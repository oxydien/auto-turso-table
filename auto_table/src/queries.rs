use std::io::{Error, ErrorKind};
use serde::Serialize;
use crate::{AtConnection, AtIntoParams, AtIntoValue, AtRows, AtValue, AutoTable, BoxError, Paginated, AtRow};
use crate::sql_helpers::{generate_insert_into, generate_order_clause, generate_select_clause, no_params};

/// The ID column is based on the [AutoTable::get_id_column] definition. By default, taking the primary key.
///
/// SELECT column_1,column_2 FROM table_name WHERE `id_column` = ?*id*
pub async fn get_by_id<T: AutoTable>(conn: &impl AtConnection, id: impl AtIntoValue) -> Result<Option<T>, BoxError> {
  let id_column = T::get_id_column();
  if let None = id_column {
    return Err(Box::new(Error::new(ErrorKind::NotSeekable, "No Id column available")));
  }

  let select_clause = generate_select_clause(&T::column_definitions());
  let query = format!("{} FROM {} WHERE {} = ? LIMIT 1", select_clause, T::table_name(), id_column.unwrap().name);

  let res = conn.query(query, vec![id]).await?;
  let formatted = format_rows::<T>(res).await?;

  Ok(formatted.into_iter().nth(0))
}

/// SELECT column_1,column_2 FROM table_name WHERE *column* = ?*value* LIMIT 1
pub async fn get_one_by_column<T: AutoTable>(conn: &impl AtConnection, column: impl ToString, value: impl AtIntoValue) -> Result<Option<T>, BoxError> {
  let select_clause = generate_select_clause(&T::column_definitions());
  let query = format!("{} FROM {} WHERE {} = ? LIMIT 1", select_clause, T::table_name(), column.to_string());

  let res = conn.query(query, vec![value]).await?;
  let formatted = format_rows::<T>(res).await?;

  Ok(formatted.into_iter().nth(0))
}

/// Returns all entries within set range.
pub async fn get_in_range<T: AutoTable>(conn: &impl AtConnection, limit: u64, offset: u64) -> Result<Vec<T>, BoxError> {
  let select_clause = generate_select_clause(&T::column_definitions());
  let order_clause = generate_order_clause(&T::get_sort_columns());
  let query = format!("{} FROM {} {} LIMIT ? OFFSET ?", select_clause, T::table_name(), order_clause);

  let res = conn.query(query, vec![limit, offset]).await?;
  let formatted = format_rows::<T>(res).await?;

  Ok(formatted)
}

/// Returns information about how many entries were found.
/// Useful for creating API interface
///
/// SELECT column_1,column_2 FROM table_name *rest_sql* ORDER BY sort_column_1 ... LIMIT *page_size* OFFSET *page_size x page*
pub async fn get_paginated<T: AutoTable + Serialize>(conn: &impl AtConnection, page: u32, page_size: u32, rest_sql: impl ToString) -> Result<Paginated<T>, BoxError> {
  let select_clause = generate_select_clause(&T::column_definitions());
  let order_clause = generate_order_clause(&T::get_sort_columns());
  let query = format!("{} FROM {} {} {} LIMIT ? OFFSET ?", select_clause, T::table_name(), rest_sql.to_string(), order_clause);

  let res = conn.query(query, vec![page_size, page_size * page]).await?;
  let formatted = format_rows::<T>(res).await?;
  
  let count_query = format!("SELECT Count(*) FROM {}", T::table_name());
  let mut count_res = conn.query(count_query, no_params()).await?;
  let count = count_res.next().await?.map(|r| r.get(0)).unwrap_or(Ok(0))?;
  
  let paginated = Paginated {
    count,
    pages: count / page_size,
    page,
    page_size,
    content: formatted,
  };

  Ok(paginated)
}

/// SELECT column_1,column_2 FROM table_name *rest_sql* [ORDER BY sort_column_1 ...]
pub async fn get_custom<T: AutoTable>(conn: &impl AtConnection, rest_sql: impl ToString, params: impl AtIntoParams, include_order: bool) -> Result<Vec<T>, BoxError> {
  let select_clause = generate_select_clause(&T::column_definitions());
  let order_clause = match include_order {
    true => {generate_order_clause(&T::get_sort_columns())}
    false => {"".to_string()}
  };
  let query = format!("{} FROM {} {} {}", select_clause, T::table_name(), rest_sql.to_string(), order_clause);

  let res = conn.query(query, params).await?;
  let formatted = format_rows::<T>(res).await?;

  Ok(formatted)
}

/// Inserts the given entry into the database. Replaces given entry if already same `primary_key` exists
pub async fn insert_into<T: AutoTable>(conn: &impl AtConnection, entry: &T) -> Result<(), BoxError> {
  let sql = generate_insert_into(T::table_name(), &T::column_definitions());
  let params = entry.to_sql_values().map_err(|e| Error::new(ErrorKind::Unsupported, format!("{:?}", e)))?;
  
  conn.execute(sql, params).await?;

  Ok(())
}

/// Automatically parses data from database into specified type [`T`] using the [`AutoTable::from_sql_values`]
pub async fn format_rows<T: AutoTable>(mut rows: impl AtRows) -> Result<Vec<T>, BoxError> {
  let mut output: Vec<T> = Vec::new();
  let mut entries: Vec<Vec<AtValue>> = Vec::new();
  while let Some(row) = rows.next().await? {
    let mut col_values: Vec<AtValue> = Vec::new();
    for col_index in 0..row.column_count() {
      col_values.push(row.get_value(col_index)?);
    }
    entries.push(col_values);
  }
  for col_values in entries {
    output.push(T::from_sql_values(col_values.as_slice()).map_err(|e|
      Box::new(Error::new(ErrorKind::NotSeekable, format!("Failed to read entry: {:?}", e))))?)
  }

  Ok(output)
}
