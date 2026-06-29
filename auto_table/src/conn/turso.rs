use std::sync::Arc;
use async_trait::async_trait;
use turso::{Connection, IntoParams, IntoValue, Row, Rows, Statement, Value};
use turso::core::types::FromValue;
use turso::params::{Params};
use crate::{AtFromValue, AtIntoParams, AtIntoValue, AtParams, AtValue, BoxError, AtCodecMarker, AtConnection, AtRows, AtStatement, AtRow};

/*
IMPLEMENTATION OF THE AUTO-TABLE ABSTRACTION LAYER
FOR turso.
 */

#[async_trait]
impl AtConnection for Connection {
  type Rows = turso::Rows;
  type Statement = turso::Statement;

  async fn query(&self, sql: impl AsRef<str> + Send, params: impl AtIntoParams) -> Result<Self::Rows, BoxError> {
    Connection::query(self, sql, Params::from(params.into_params()?)).await.map_err(|e| e.into())
  }

  async fn execute(&self, sql: impl AsRef<str> + Send, params: impl AtIntoParams) -> Result<u64, BoxError> {
    Connection::execute(self, sql, Params::from(params.into_params()?)).await.map_err(|e| e.into())
  }

  async fn prepare(&self, sql: impl AsRef<str> + Send) -> Result<Self::Statement, BoxError> {
    Connection::prepare(self, sql).await.map_err(|e| e.into())
  }
}

#[async_trait]
impl AtConnection for Arc<Connection> {
  type Rows = turso::Rows;
  type Statement = turso::Statement;

  async fn query(&self, sql: impl AsRef<str> + Send, params: impl AtIntoParams) -> Result<Self::Rows, BoxError> {
    Connection::query(self, sql, Params::from(params.into_params()?)).await.map_err(|e| e.into())
  }

  async fn execute(&self, sql: impl AsRef<str> + Send, params: impl AtIntoParams) -> Result<u64, BoxError> {
    Connection::execute(self, sql, Params::from(params.into_params()?)).await.map_err(|e| e.into())
  }

  async fn prepare(&self, sql: impl AsRef<str> + Send) -> Result<Self::Statement, BoxError> {
    Connection::prepare(self, sql).await.map_err(|e| e.into())
  }
}


#[async_trait]
impl AtRows for Rows {
  type Row = turso::Row;

  fn column_count(&self) -> usize {
    self.column_count()
  }

  async fn next(&mut self) -> Result<Option<Self::Row>, BoxError> {
    self.next().await.map_err(|e| e.into())
  }
}

impl AtRow for Row {
  fn get_value(&self, idx: usize) -> Result<AtValue, BoxError> {
    self.get_value(idx).map(|v| v.into()).map_err(|e| e.into())
  }

  fn get<T>(&self, idx: usize) -> Result<T, BoxError>
  where
    T: AtFromValue
  {
    let raw_val: turso::core::Value = self.get(idx).map_err(|e| Box::new(e) as BoxError)?;
    let val: turso::Value = raw_val.into();
    T::from_sql(val.into())
  }

  fn column_count(&self) -> usize {
    self.column_count()
  }
}

#[async_trait]
impl AtStatement for Statement {
  type Rows = turso::Rows;

  async fn query(&mut self, params: impl AtIntoParams) -> Result<Self::Rows, BoxError> {
    self.query(Params::from(params.into_params()?)).await.map_err(|e| e.into())
  }
}

impl From<AtParams> for Params
{
  fn from(value: AtParams) -> Self {
    match value {
      AtParams::None => Params::None,
      AtParams::Positional(p) => Params::Positional(p.into_iter().map(|e| e.into()).collect()),
      AtParams::Named(n) => Params::Named(n.into_iter().map(|e| (e.0, e.1.into())).collect()),
    }
  }
}

impl IntoValue for AtValue {
  fn into_value(self) -> turso::Result<Value> {
    Ok(self.into())
  }
}

impl AtIntoValue for Value {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(self.into())
  }
}

impl From<Value> for AtValue {
  fn from(value: Value) -> Self {
    match value {
      Value::Null => AtValue::Null,
      Value::Integer(i) => AtValue::Integer(i),
      Value::Real(r) => AtValue::Real(r),
      Value::Text(t) => AtValue::Text(t),
      Value::Blob(b) => AtValue::Blob(b),
    }
  }
}

impl From<AtValue> for Value {
  fn from(value: AtValue) -> Self {
    match value {
      AtValue::Null => Value::Null,
      AtValue::Integer(i) => Value::Integer(i),
      AtValue::Real(r) => Value::Real(r),
      AtValue::Text(t) => Value::Text(t),
      AtValue::Blob(b) => Value::Blob(b),
    }
  }
}

impl<T> AtIntoParams for T
where
  T: IntoParams + Send + AtCodecMarker,
{
  fn into_params(self) -> Result<AtParams, BoxError> {
    self.into_params().map(|val| match val {
      Params::None => AtParams::None,
      Params::Positional(p) => AtParams::Positional(p.into_iter().map(|e| e.into()).collect()),
      Params::Named(n) => AtParams::Named(n.into_iter().map(|p| (p.0, p.1.into())).collect()),
    }).map_err(|e| e.into())
  }
}

impl<T> AtFromValue for T
where
  T: FromValue + Send + AtCodecMarker,
{
  fn from_sql(val: AtValue) -> Result<Self, BoxError>
  where
    Self: Sized
  {
    let t_val = IntoValue::into_value(val)?;
    Self::from_sql(t_val.into())
      .map_err(|e| e.into())
  }
}
