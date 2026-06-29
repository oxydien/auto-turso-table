use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Field, Fields, LitStr, Type};

// Mark: Attribute containers

struct StructAttrs {
    /// If not set, the name of the table is taken from the struct name
    table_name: Option<String>,
    /// Format: <column_name> <ASC/DESC>, [...]
    table_indexes: Option<String>,
}

struct FieldAttrs {
    primary_key: bool,
    sort_asc: bool,
    sort_desc: bool,
    data_type: Option<String>,
    /// (to_sql_fn, from_sql_fn)
    with: Option<(String, String)>,
    default: Option<String>, // stored as SQL literal
}

// Mark: Entry point

#[proc_macro_derive(AutoTable, attributes(auto_table))]
pub fn derive_auto_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match impl_auto_table(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// Mark: Code generation

fn impl_auto_table(input: &DeriveInput) -> Result<TokenStream2, syn::Error> {
    let struct_name = &input.ident;
    let s_attrs = parse_struct_attrs(input)?;

    let table_name = s_attrs
        .table_name
        .unwrap_or_else(|| format!("{}s", to_snake_case(&struct_name.to_string())));

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "AutoTable: only named fields are supported",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input,
                "AutoTable: only structs are supported",
            ));
        }
    };

    let table_indexes = s_attrs.table_indexes.unwrap_or(String::new());

    let col_exprs: Vec<TokenStream2> = fields
        .iter()
        .map(generate_col_expr)
        .collect::<Result<_, _>>()?;

    let to_sql_body = generate_to_sql_body(fields)?;
    let from_sql_body = generate_from_sql_body(fields)?;

    Ok(quote! {
        impl auto_table::AutoTable for #struct_name {

            fn table_name() -> &'static str { #table_name }

            fn indexes() -> Option<String> {
                let idxs = #table_indexes;
                if idxs.is_empty() { None } else { Some(idxs.to_string()) }
            }

            fn column_definitions() -> ::std::vec::Vec<auto_table::ColumnDefinition> {
                vec![ #(#col_exprs),* ]
            }

            fn to_sql_values(&self) -> Result<::std::vec::Vec<::auto_table::AtValue>, auto_table::SqlError> {
                #to_sql_body
            }

            fn from_sql_values(values: &[::auto_table::AtValue]) -> Result<Self, auto_table::SqlError> {
                #from_sql_body
            }
        }
    })
}

/// Generates `ColumnDefinition` for a field
fn generate_col_expr(field: &Field) -> Result<TokenStream2, syn::Error> {
    let col_name = field.ident.as_ref().unwrap().to_string();
    let f_attrs = parse_field_attrs(field)?;
    let (base_ty, is_option) = extract_option_inner(&field.ty);

    let sql_type_expr = if let Some(data_type) = f_attrs.data_type {
        quote! { #data_type.to_string() }
    } else {
        let t = map_type_to_sql(base_ty, field)?;
        quote! { #t.to_string() }
    };

    let nullable = is_option;
    let pk = f_attrs.primary_key;
    let default_expr = match &f_attrs.default {
        Some(s) => quote! { ::std::option::Option::Some(#s.to_string()) },
        None => quote! { ::std::option::Option::None },
    };
    let sort_expr = if f_attrs.sort_desc {
        quote! { ::std::option::Option::Some("DESC".to_string()) }
    } else if f_attrs.sort_asc {
        quote! { ::std::option::Option::Some("ASC".to_string()) }
    } else {
        quote! { ::std::option::Option::None }
    };

    Ok(quote! {
        auto_table::ColumnDefinition {
            name: #col_name,
            sql_type: #sql_type_expr,
            nullable: #nullable,
            primary_key: #pk,
            sort_by: #sort_expr,
            default: #default_expr,
        }
    })
}

/// Generates the body of `to_sql_values()`.
///
/// Priority: `with` > built-in type > `AtSqlCodec`.
/// `Option<T>` always maps `None` to the string `"NULL"`.
/// `bool` always maps to `"1"` / `"0"` rather than `"true"` / `"false"`.
fn generate_to_sql_body(
    fields: &syn::punctuated::Punctuated<Field, syn::token::Comma>,
) -> Result<TokenStream2, syn::Error> {
    let mut conversions = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let f_attrs = parse_field_attrs(field)?;
        let (base_ty, is_option) = extract_option_inner(&field.ty);

        let into_value_match = generate_into_sql_match();

        let conversion = if let Some((to_sql_fn, _)) = &f_attrs.with {
            // `with` has the highest priority; the user handles Option themselves.
            let _fn_path: syn::Path = syn::parse_str(to_sql_fn)?;
            quote! {{
                fn_path(&self.#field_name)
            }}
        } else if is_built_in_sql_type(base_ty) {
            // bool must emit "1"/"0" so SQLite's INTEGER affinity round-trips correctly.
            let is_bool = is_simple_ident(base_ty, "bool");
            if is_option {
                let some_arm = if is_bool {
                    quote! {
                        if *v { match "1".#into_value_match } else { match "0".#into_value_match }
                    }
                } else {
                    quote! {
                        match v.clone().#into_value_match
                    }
                };
                quote! {
                    match &self.#field_name {
                        Some(v) => #some_arm,
                        None    => match "NULL".#into_value_match,
                    }
                }
            } else if is_bool {
                quote! {
                    if self.#field_name { match "1".#into_value_match } else { match "0".#into_value_match }
                }
            } else {
                quote! {
                    match self.#field_name.clone().#into_value_match
                }
            }
        } else {
            // Fall back to AtSqlCodec. A compile error here means the type neither has a
            // built-in mapping nor implements AtSqlCodec (and no `with` was supplied).
            if is_option {
                quote! {
                    match &self.#field_name {
                        Some(v) => {
                          match <#base_ty as auto_table::AtSqlCodec>::to_sql(v).#into_value_match
                        },
                        None    => "NULL".to_string(),
                    }
                }
            } else {
                quote! {
                    match <#base_ty as auto_table::AtSqlCodec>::to_sql(&self.#field_name).#into_value_match
                }
            }
        };

        conversions.push(conversion);
    }

    Ok(quote! {
        Ok(vec![ #(#conversions),* ])
    })
}

fn generate_into_sql_match() -> TokenStream2 {
  quote! {
    into_value() {
      Ok(l) => l,
      Err(why) => return Err(SqlError::SerializationIssue(why.to_string()))
    }
  }
}

/// Generates the body of `from_sql_values()`.
///
/// Priority: `with` > built-in type > `AtSqlCodec`.
fn generate_from_sql_body(
    fields: &syn::punctuated::Punctuated<Field, syn::token::Comma>,
) -> Result<TokenStream2, syn::Error> {
    let mut field_assignments = Vec::new();
    let mut index = 0usize;

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let f_attrs = parse_field_attrs(field)?;
        let (base_ty, is_option) = extract_option_inner(&field.ty);
        let idx = syn::Index::from(index);

        let assignment = if let Some((_, from_sql_fn)) = &f_attrs.with {
            let fn_path: syn::Path = syn::parse_str(from_sql_fn)?;
            quote! {
                #field_name: {
                    let span = ::tracing::span!(::tracing::Level::TRACE, "parsing");
                    let _enter = span.enter();
                    ::tracing::trace!(
                        target: "parsing",
                        field = #field_name_str,
                        value = ?&values[#idx],
                        converter = #from_sql_fn,
                        "Parsing field with custom converter"
                    );
                    #fn_path(&values[#idx])?
                }
            }
        } else if is_built_in_sql_type(base_ty) {
            let extraction = generate_value_extraction(base_ty, &idx, is_option, field)?;
            quote! { #field_name: #extraction }
        } else {
            // AtSqlCodec fallback
            let extraction =
                generate_at_sql_codec_extraction(base_ty, &idx, is_option, &field_name_str);
            quote! { #field_name: #extraction }
        };

        field_assignments.push(assignment);
        index += 1;
    }

    let struct_name = fields.iter().next().map(|_| quote! { Self });

    Ok(quote! {
        if values.len() != #index {
            return Err(auto_table::SqlError::ColumnCountMismatch);
        }
        Ok(#struct_name {
            #(#field_assignments),*
        })
    })
}

/// Generates extraction code for types with a built-in `Value` mapping.
/// For nullable fields, also matches `Value::Text("NULL")` so that values
/// written by `to_sql_values` (which emits the string `"NULL"` for `None`)
/// round-trip correctly.
fn generate_value_extraction(
    ty: &Type,
    idx: &syn::Index,
    nullable: bool,
    field: &Field,
) -> Result<TokenStream2, syn::Error> {
    let field_name = field.ident.as_ref().unwrap().to_string();
    let (pattern, expr) = match_arm_for_type(ty, field)?;

    if nullable {
        Ok(quote! {{
            let span = ::tracing::span!(::tracing::Level::TRACE, "parsing");
            let _enter = span.enter();
            ::tracing::trace!(
                target: "parsing",
                field = #field_name,
                value = ?&values[#idx],
                "Parsing nullable field"
            );
            match &values[#idx] {
                ::auto_table::AtValue::Null => None,
                ::auto_table::AtValue::Text(__s) if __s.as_str() == "NULL" => None,
                #pattern => Some(#expr),
                _ => return Err(auto_table::SqlError::InvalidData(#field_name.to_string())),
            }
        }})
    } else {
        Ok(quote! {{
            let span = ::tracing::span!(::tracing::Level::TRACE, "parsing");
            let _enter = span.enter();
            ::tracing::trace!(
                target: "parsing",
                field = #field_name,
                value = ?&values[#idx],
                "Parsing required field"
            );
            match &values[#idx] {
                #pattern => #expr,
                _ => return Err(auto_table::SqlError::InvalidData(#field_name.to_string())),
            }
        }})
    }
}

/// Generates extraction code that delegates to `AtSqlCodec::from_sql`.
/// Nullable fields treat both `Value::Null` and the string `"NULL"` as `None`.
fn generate_at_sql_codec_extraction(
    base_ty: &Type,
    idx: &syn::Index,
    nullable: bool,
    field_name: &str,
) -> TokenStream2 {
    if nullable {
        quote! {{
            let span = ::tracing::span!(::tracing::Level::TRACE, "parsing");
            let _enter = span.enter();
            ::tracing::trace!(
                target: "parsing",
                field = #field_name,
                value = ?&values[#idx],
                "Parsing nullable AtSqlCodec field"
            );
            match &values[#idx] {
                ::auto_table::AtValue::Null => None,
                ::auto_table::AtValue::Text(__s) if __s.as_str() == "NULL" => None,
                __v => Some(<#base_ty as auto_table::AtSqlCodec>::from_sql(__v)?),
            }
        }}
    } else {
        quote! {{
            let span = ::tracing::span!(::tracing::Level::TRACE, "parsing");
            let _enter = span.enter();
            ::tracing::trace!(
                target: "parsing",
                field = #field_name,
                value = ?&values[#idx],
                "Parsing AtSqlCodec field"
            );
            <#base_ty as auto_table::AtSqlCodec>::from_sql(&values[#idx])?
        }}
    }
}

/// Given a base Rust type (already unwrapped from `Option<T>` if needed),
/// returns the `(pattern, expression)` pair for a `match &values[idx]` arm.
fn match_arm_for_type(
    ty: &Type,
    field: &Field,
) -> Result<(TokenStream2, TokenStream2), syn::Error> {
    match ty {
        Type::Path(tp) => {
            let seg = tp
                .path
                .segments
                .last()
                .ok_or_else(|| syn::Error::new_spanned(field, "empty type path"))?;
            let ident = &seg.ident;
            match seg.ident.to_string().as_str() {
                "bool" => Ok((quote!(auto_table::AtValue::Integer(i)), quote!(*i != 0))),
                "i64" => Ok((quote!(auto_table::AtValue::Integer(i)), quote!(*i))),
                "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i128" | "usize"
                | "isize" => Ok((
                    quote!(auto_table::AtValue::Integer(i)),
                    quote!(*i as #ident),
                )),
                "f64" => Ok((quote!(auto_table::AtValue::Real(r)), quote!(*r))),
                "f32" => Ok((quote!(auto_table::AtValue::Real(r)), quote!(*r as f32))),
                "String" => Ok((quote!(auto_table::AtValue::Text(s)), quote!(s.clone()))),
                "Vec" => {
                    if is_vec_u8(tp) {
                        Ok((quote!(auto_table::AtValue::Blob(b)), quote!(b.clone())))
                    } else {
                        Err(syn::Error::new_spanned(
                            field,
                            "Vec<T> is only supported as BLOB (Vec<u8>); \
                             use `#[auto_table(with = \"to_fn, from_fn\")]`",
                        ))
                    }
                }
                other => Err(syn::Error::new_spanned(
                    field,
                    format!(
                        "`{}` has no automatic Value conversion; implement `AtSqlCodec` \
                     and add `#[auto_table(data_type = \"SQL TYPE\")]`, or use \
                     `#[auto_table(with = \"to_fn, from_fn\")]`",
                        other
                    ),
                )),
            }
        }
        _ => Err(syn::Error::new_spanned(
            field,
            "unsupported type for Value conversion",
        )),
    }
}

// Mark: Attribute parsing

fn parse_struct_attrs(input: &DeriveInput) -> Result<StructAttrs, syn::Error> {
    let mut attrs = StructAttrs {
        table_name: None,
        table_indexes: None,
    };
    for attr in &input.attrs {
        if !attr.path().is_ident("auto_table") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                let s: LitStr = meta.value()?.parse()?;
                attrs.table_name = Some(s.value());
            } else if meta.path.is_ident("index_by") {
                let s: LitStr = meta.value()?.parse()?;
                attrs.table_indexes = Some(s.value());
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

fn parse_field_attrs(field: &Field) -> Result<FieldAttrs, syn::Error> {
    let mut attrs = FieldAttrs {
        primary_key: false,
        sort_asc: false,
        sort_desc: false,
        data_type: None,
        with: None,
        default: None,
    };

    for attr in &field.attrs {
        if !attr.path().is_ident("auto_table") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("primary_key") {
                attrs.primary_key = true;
            } else if meta.path.is_ident("sort_asc") {
                attrs.sort_asc = true;
            } else if meta.path.is_ident("sort_desc") {
                attrs.sort_desc = true;
            } else if meta.path.is_ident("data_type") {
                let lit: syn::Lit = meta.value()?.parse()?;
                if let syn::Lit::Str(lit) = lit {
                    attrs.data_type = Some(lit.value());
                }
            } else if meta.path.is_ident("with") {
                let s: syn::LitStr = meta.value()?.parse()?;
                let funcs = s.value();
                let parts: Vec<&str> = funcs.split(',').map(|p| p.trim()).collect();
                if parts.len() == 2 {
                    attrs.with = Some((parts[0].to_string(), parts[1].to_string()));
                } else {
                    return Err(meta.error("expected `with = \"to_sql_fn, from_sql_fn\"`"));
                }
            } else if meta.path.is_ident("default") {
                let lit: syn::Lit = meta.value()?.parse()?;
                attrs.default = Some(match &lit {
                    syn::Lit::Str(s) => format!("'{}'", s.value()),
                    syn::Lit::Int(i) => i.base10_digits().to_string(),
                    syn::Lit::Float(f) => f.base10_digits().to_string(),
                    syn::Lit::Bool(b) => if b.value { "1" } else { "0" }.to_string(),
                    _ => return Err(meta.error("unsupported default literal")),
                });
            } else {
                return Err(meta.error(format!(
                    "unknown auto_table attribute: `{}`",
                    meta.path
                        .get_ident()
                        .map_or(String::new(), |w| w.to_string())
                )));
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

// Mark: Type helpers

/// Peels one layer of `Option<T>`, returning `(inner_type, true)` if present.
fn extract_option_inner(ty: &Type) -> (&Type, bool) {
    if let Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = ab.args.first() {
                        return (inner, true);
                    }
                }
            }
        }
    }
    (ty, false)
}

/// Returns `true` for types that have a built-in SQL / Value mapping
/// (integers, floats, `String`, `Vec<u8>`, `&str`).
/// Unknown types should use `AtSqlCodec` or `with`.
fn is_built_in_sql_type(ty: &Type) -> bool {
    match ty {
        Type::Path(tp) => {
            let seg = match tp.path.segments.last() {
                Some(s) => s,
                None => return false,
            };
            match seg.ident.to_string().as_str() {
                "bool" | "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64"
                | "i128" | "usize" | "isize" | "f32" | "f64" | "String" => true,
                "Vec" => is_vec_u8(tp),
                _ => false,
            }
        }
        Type::Reference(r) => {
            if let Type::Path(tp) = &*r.elem {
                tp.path.is_ident("str")
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Returns `true` if `ty` is a simple path type whose last segment matches `name`.
fn is_simple_ident(ty: &Type, name: &str) -> bool {
    if let Type::Path(tp) = ty {
        tp.path.is_ident(name)
    } else {
        false
    }
}

/// Maps a Rust type to a SQLite affinity string.
/// Returns a compile error for types with no automatic mapping;
/// those should use `AtSqlCodec` (pair with `data_type`) or `with`.
fn map_type_to_sql(ty: &Type, field: &Field) -> Result<&'static str, syn::Error> {
    match ty {
        Type::Path(tp) => {
            let seg = tp
                .path
                .segments
                .last()
                .ok_or_else(|| syn::Error::new_spanned(field, "empty type path"))?;
            match seg.ident.to_string().as_str() {
                "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128"
                | "usize" | "isize" | "bool" => Ok("INTEGER"),
                "f32" | "f64" => Ok("REAL"),
                "String" => Ok("TEXT"),
                "Vec" => {
                    if is_vec_u8(tp) {
                        Ok("BLOB")
                    } else {
                        Err(syn::Error::new_spanned(
                            field,
                            "Vec<T> is only supported as BLOB (Vec<u8>); \
                             use `#[auto_table(with = \"to_fn, from_fn\")]`",
                        ))
                    }
                }
                "Option" => {
                    let val = get_option_type(tp);
                    if val == "UNKNOWN" {
                        Err(syn::Error::new_spanned(
                            field,
                            "Option<?> has no automatic SQL mapping; \
                             use `#[auto_table(data_type = \"SQL TYPE\")]`",
                        ))
                    } else {
                        Ok(val)
                    }
                }
                other => Err(syn::Error::new_spanned(
                    field,
                    format!(
                        "`{}` has no automatic SQL mapping; implement `AtSqlCodec` and add \
                     `#[auto_table(data_type = \"SQL TYPE\")]`, or use \
                     `#[auto_table(with = \"to_fn,from_fn\")]`",
                        other
                    ),
                )),
            }
        }
        Type::Reference(r) => {
            if let Type::Path(tp) = &*r.elem {
                if tp.path.is_ident("str") {
                    return Ok("TEXT");
                }
            }
            Err(syn::Error::new_spanned(
                field,
                "only `&str` is supported as a reference type",
            ))
        }
        _ => Err(syn::Error::new_spanned(field, "unsupported type syntax")),
    }
}

fn is_vec_u8(tp: &syn::TypePath) -> bool {
    let seg = match tp.path.segments.last() {
        Some(s) => s,
        None => return false,
    };
    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
        if let Some(syn::GenericArgument::Type(Type::Path(p))) = ab.args.first() {
            return p.path.is_ident("u8");
        }
    }
    false
}

fn get_option_type(tp: &syn::TypePath) -> &'static str {
    let seg = match tp.path.segments.last() {
        Some(s) => s,
        None => return "UNKNOWN",
    };
    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
        if let Some(syn::GenericArgument::Type(Type::Path(p))) = ab.args.first() {
            return p.path.get_ident().map_or("UNKNOWN", |ident| {
                match ident.to_string().as_str() {
                    "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64"
                    | "i128" | "usize" | "isize" | "bool" => "INTEGER",
                    "f32" | "f64" => "REAL",
                    "String" => "TEXT",
                    _ => "UNKNOWN",
                }
            });
        }
    }
    "UNKNOWN"
}

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}
