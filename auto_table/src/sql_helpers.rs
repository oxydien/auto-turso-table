use crate::{AtConnection, BoxError, ColumnDefinition, AtStatement, AtRows, AtRow};

pub(crate) async fn get_columns(conn: &impl AtConnection, table_name: &str) -> Result<Vec<String>, BoxError> {
  let validated_table_name = validate_table_name(table_name)?;
  let query = format!("PRAGMA table_info({})", validated_table_name);
  let mut smtp = conn.prepare(query).await.expect("Failed to prepare query");
  let mut rows = smtp.query(no_params()).await.expect("Failed to query PRAGMA table_info");
  let mut columns = vec![];
  while let Some(row) = rows.next().await? {
    let name: String = row.get(1)?;
    columns.push(name);
  }
  Ok(columns)
}

pub fn generate_create_table(table_name: &str, columns: &Vec<ColumnDefinition>) -> String {
  let cols_sql = columns.iter()
                         .map(|c| c.to_sql())
                         .collect::<Vec<_>>()
                         .join(", ");
   format!("CREATE TABLE {} ({})", table_name, cols_sql)
}

pub fn generate_select_clause(columns: &Vec<ColumnDefinition>) -> String {
  let mut query = String::from("SELECT ");
  for col in columns {
    query.push_str(&col.name);
    query.push(',');
  }
  query.pop();
  query
}

pub fn generate_order_clause(sortable_columns: &Vec<ColumnDefinition>) -> String {
  if sortable_columns.len() == 0 { return String::new(); }

  let mut builder = String::from("ORDER BY ");
  for col in sortable_columns {
    builder.push_str(&col.name);
    if let Some(sort_direction) = &col.sort_by {
      builder.push(' ');
      builder.push_str(sort_direction);
    }
    builder.push(',');
  }
  builder.pop();
  builder
}

pub fn generate_insert_into(table_name: &str, columns: &Vec<ColumnDefinition>) -> String {
  let mut query = format!("INSERT OR REPLACE INTO {} (", table_name);

  for col in columns {
    query.push_str(&col.name);
    query.push(',');
  }
  query.pop();

  query.push_str(") VALUES (");
  for _ in 0..columns.len() {
    query.push('?');
    query.push(',');
  }
  query.pop();
  query.push(')');

  query
}

pub fn no_params() -> Vec<u8> {
  vec![]
}

pub(crate) fn validate_table_name(name: &str) -> Result<&str, String> {
  if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
    Ok(name)
  } else {
    Err("Invalid table name".to_string())
  }
}
