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
    sort_desc: bool,
    data_type: Option<String>,
    /// (to_sql_fn, from_sql_fn)
    with: Option<(String, String)>,
    default: Option<String>,        // stored as SQL literal
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

    // Default table name: snake_case + "s"  (e.g. Message → messages)
    let table_name = s_attrs
        .table_name
        .unwrap_or_else(|| format!("{}s", to_snake_case(&struct_name.to_string())));

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => return Err(syn::Error::new_spanned(input, "AutoTable: only named fields are supported")),
        },
        _ => return Err(syn::Error::new_spanned(input, "AutoTable: only structs are supported")),
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
            fn table_name() -> &'static str {
                #table_name
            }

            fn indexes() -> Option<String> {
                let idxs = #table_indexes;
                if idxs.is_empty() {None} else {Some(idxs.to_string())}
            }

            fn column_definitions() -> ::std::vec::Vec<auto_table::ColumnDefinition> {
                vec![ #(#col_exprs),* ]
            }

            fn to_sql_values(&self) -> ::std::vec::Vec<::std::string::String> {
                #to_sql_body
            }

            fn from_sql_values(values: &[::turso::value::Value]) -> Result<Self, auto_table::SqlError> {
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
        quote! {
            #data_type.to_string()
        }
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

    let sort_expr = match &f_attrs.sort_desc {
        &true => quote!{::std::option::Option::Some("DESC".to_string())},
        &false => quote!{ ::std::option::Option::None }
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

/// Generates the body of `to_sql_values()` method
fn generate_to_sql_body(fields: &syn::punctuated::Punctuated<Field, syn::token::Comma>) -> Result<TokenStream2, syn::Error> {
    let mut conversions = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let f_attrs = parse_field_attrs(field)?;

        let conversion = if let Some((to_sql_fn, _)) = &f_attrs.with {
            let fn_path: syn::Path = syn::parse_str(to_sql_fn)?;
            quote! {
                {
                    let sql_value = #fn_path(&self.#field_name);
                    sql_value.to_string()
                }
            }
        } else {
            quote! { self.#field_name.to_string() }
        };

        conversions.push(conversion);
    }

    Ok(quote! {
        vec![ #(#conversions),* ]
    })
}

/// Generates the body of `from_sql_values()` method
fn generate_from_sql_body(fields: &syn::punctuated::Punctuated<Field, syn::token::Comma>) -> Result<TokenStream2, syn::Error> {
    let mut field_assignments = Vec::new();
    let mut index = 0usize;

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.clone().to_string();
        let f_attrs = parse_field_attrs(field)?;
        let idx = syn::Index::from(index);

        let assignment = if let Some((_, from_sql_fn)) = &f_attrs.with {
            let fn_path: syn::Path = syn::parse_str(from_sql_fn)?;
            quote! {
                #field_name: {
                    ::tracing::trace!(
                        field = #field_name_str,
                        value = ?&values[#idx],
                        converter = #from_sql_fn,
                        "Parsing field with custom converter"
                    );
                    #fn_path(&values[#idx])?
                }
            }
        } else {
            let (base_ty, is_option) = extract_option_inner(&field.ty);
            let extraction = generate_value_extraction(base_ty, &idx, is_option, field)?;
            quote! {
                #field_name: #extraction
            }
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

/// Given a base Rust type (already unwrapped from Option<T> if needed),
/// returns the `(pattern, expression)` pair for a `match &values[idx]` arm.
///
/// e.g. for `u32` → (`Value::Integer(i)`, `*i as u32`)
///      for `String` → (`Value::Text(s)`, `s.clone()`)
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
                // bool is stored as INTEGER 0/1
                "bool" => Ok((
                    quote!(turso::value::Value::Integer(i)),
                    quote!(*i != 0),
                )),
                // i64 is the native INTEGER type — no cast needed
                "i64" => Ok((
                    quote!(turso::value::Value::Integer(i)),
                    quote!(*i),
                )),
                // All other integer types cast from i64
                "u8" | "u16" | "u32" | "u64" | "u128"
                | "i8" | "i16" | "i32" | "i128"
                | "usize" | "isize" => Ok((
                    quote!(turso::value::Value::Integer(i)),
                    quote!(*i as #ident),
                )),
                // f64 is the native REAL type
                "f64" => Ok((
                    quote!(turso::value::Value::Real(r)),
                    quote!(*r),
                )),
                "f32" => Ok((
                    quote!(turso::value::Value::Real(r)),
                    quote!(*r as f32),
                )),
                "String" => Ok((
                    quote!(turso::value::Value::Text(s)),
                    quote!(s.clone()),
                )),
                "Vec" => {
                    if is_vec_u8(tp) {
                        Ok((
                            quote!(turso::value::Value::Blob(b)),
                            quote!(b.clone()),
                        ))
                    } else {
                        Err(syn::Error::new_spanned(
                            field,
                            "Vec<T> is only supported as BLOB (Vec<u8>); \
                             use #[auto_table(with = \"...\")]",
                        ))
                    }
                }
                other => Err(syn::Error::new_spanned(
                    field,
                    format!(
                        "`{}` has no automatic Value conversion; \
                         use #[auto_table(with = \"fn_name\")]",
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

/// Generates the full extraction expression for one field, handling
/// both `Option<T>` (nullable) and bare `T` (non-nullable).
fn generate_value_extraction(
    ty: &Type,
    idx: &syn::Index,
    nullable: bool,
    field: &Field,
) -> Result<TokenStream2, syn::Error> {
    let field_name = field.ident.as_ref().unwrap().to_string();
    let (pattern, expr) = match_arm_for_type(ty, field)?;

    if nullable {
        // Option<T>: Null → None, matching variant → Some(value)
        Ok(quote! {
            {
                ::tracing::trace!(
                        field = #field_name,
                        value = ?&values[#idx],
                        "Parsing nullable field"
                );
                match &values[#idx] {
                    ::turso::value::Value::Null => None,
                    #pattern => Some(#expr),
                    _ => return Err(auto_table::SqlError::InvalidData(#field_name.to_string())),
                }
            }
        })
    } else {
        Ok(quote! {
            {
                ::tracing::trace!(
                    field = #field_name,
                    value = ?&values[#idx],
                    "Parsing required field"
                );
                match &values[#idx] {
                    #pattern => #expr,
                    _ => return Err(auto_table::SqlError::InvalidData(#field_name.to_string())),
                }
            }
        })
    }
}

// Mark: Attribute parsing

fn parse_struct_attrs(input: &DeriveInput) -> Result<StructAttrs, syn::Error> {
    let mut attrs = StructAttrs { table_name: None, table_indexes: None };
    for attr in &input.attrs {
        if !attr.path().is_ident("auto_table") { continue; }
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
    let mut attrs = FieldAttrs { primary_key: false, sort_desc: false, data_type: None, with: None, default: None };

    for attr in &field.attrs {
        if !attr.path().is_ident("auto_table") { continue; }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("primary_key") {
                attrs.primary_key = true;
            } else if meta.path.is_ident("sort_desc") {
                attrs.sort_desc = true;
            } else if meta.path.is_ident("data_type") {
                let lit: syn::Lit = meta.value()?.parse()?;
                if let syn::Lit::Str(lit) = lit {
                    attrs.data_type = Some(lit.value());
                }
            } else if meta.path.is_ident("with") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                let funcs = s.value();
                // Expected format: "to_sql_fn, from_sql_fn" or "module::to_sql, module::from_sql"
                let parts: Vec<&str> = funcs.split(',').map(|p| p.trim()).collect();
                if parts.len() == 2 {
                    attrs.with = Some((parts[0].to_string(), parts[1].to_string()));
                } else {
                    return Err(meta.error("expected `with = \"to_sql_fn, from_sql_fn\"`"))
                }
            } else if meta.path.is_ident("default") {
                let lit: syn::Lit = meta.value()?.parse()?;
                // Store default as its SQL literal representation
                attrs.default = Some(match &lit {
                    syn::Lit::Str(s)   => format!("'{}'", s.value()),
                    syn::Lit::Int(i)   => i.base10_digits().to_string(),
                    syn::Lit::Float(f) => f.base10_digits().to_string(),
                    syn::Lit::Bool(b)  => if b.value { "1" } else { "0" }.to_string(),
                    _ => return Err(meta.error("unsupported default literal")),
                });
            } else {
                return Err(meta.error(format!("unknown auto_table attribute {}",
                      meta.path.get_ident().map_or(String::new(), |w| w.to_string()))));
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

// Mark: Type helpers

/// Peels one layer of `Option<T>`, returning (inner_type, true) if present.
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

/// Maps a Rust type to a SQLite affinity string.
/// Returns a compile error if the type is unrecognized (use `with = "..."` in that case).
fn map_type_to_sql(ty: &Type, field: &Field) -> Result<&'static str, syn::Error> {
    match ty {
        Type::Path(tp) => {
            let seg = tp.path.segments.last()
                .ok_or_else(|| syn::Error::new_spanned(field, "empty type path"))?;
            match seg.ident.to_string().as_str() {
                "u8"  | "u16"   | "u32"   | "u64"  | "u128"
                | "i8"  | "i16"   | "i32"   | "i64"  | "i128"
                | "usize" | "isize" | "bool" => Ok("INTEGER"),
                "f32" | "f64" => Ok("REAL"),
                "String"      => Ok("TEXT"),
                "Vec" => {
                    if is_vec_u8(tp) { Ok("BLOB") } else {
                        Err(syn::Error::new_spanned(field, "Vec<T> is only supported as BLOB (Vec<u8>); \
                             use #[auto_table(with = \"...\")]"))
                    }
                }
                "Option" => {
                    let val = get_option_type(tp);
                    if val == "UNKNOWN" {
                        Err(syn::Error::new_spanned(field, "Option<?> has no automatic SQL mapping; \
                             use #[auto_table(data_type = \"SQL DATA TYPE HERE\")]".to_string()))
                    } else {
                        Ok(val)
                    }
                }
                other => Err(syn::Error::new_spanned(field, format!(
                    "`{}` has no automatic SQL mapping; \
                     use #[auto_table(data_type = \"SQL DATA TYPE HERE\")]", other
                ))),
            }
        }
        Type::Reference(r) => {
            if let Type::Path(tp) = &*r.elem {
                if tp.path.is_ident("str") { return Ok("TEXT"); }
            }
            Err(syn::Error::new_spanned(field, "only &str is supported as a reference type"))
        }
        _ => Err(syn::Error::new_spanned(field, "unsupported type syntax")),
    }
}

fn is_vec_u8(tp: &syn::TypePath) -> bool {
    let seg = match tp.path.segments.last() { Some(s) => s, None => return false };
    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
        if let Some(syn::GenericArgument::Type(Type::Path(p))) = ab.args.first() {
            return p.path.is_ident("u8");
        }
    }
    false

}

fn get_option_type(tp: &syn::TypePath) -> &'static str {
    let seg = match tp.path.segments.last() { Some(s) => s, None => return "UNKNOWN" };
    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
        if let Some(syn::GenericArgument::Type(Type::Path(p))) = ab.args.first() {
            return p.path.get_ident().map_or("UNKNOWN", |indent| {
                match indent.to_string().as_str() {
                    "u8"  | "u16"   | "u32"   | "u64"  | "u128"
                    | "i8"  | "i16"   | "i32"   | "i64"  | "i128"
                    | "usize" | "isize" | "bool" => "INTEGER",
                    "f32" | "f64" => "REAL",
                    "String"      => "TEXT",
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
        if c.is_uppercase() && i > 0 { out.push('_'); }
        out.extend(c.to_lowercase());
    }
    out
}
