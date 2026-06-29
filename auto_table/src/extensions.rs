use crate::{AtCodecMarker, AtIntoParams, AtIntoValue, AtParams, AtValue, BoxError};

impl<T: AtIntoValue, const N: usize> AtCodecMarker for [T; N] {}
impl<T: AtIntoValue>  AtCodecMarker for Vec<(String, T)> {}
impl  AtCodecMarker for u128 {}
impl  AtCodecMarker for u64 {}
impl  AtCodecMarker for u32 {}
impl  AtCodecMarker for u16 {}
impl  AtCodecMarker for u8 {}
impl  AtCodecMarker for i128 {}
impl  AtCodecMarker for i64 {}
impl  AtCodecMarker for i32 {}
impl  AtCodecMarker for i16 {}
impl  AtCodecMarker for i8 {}
impl  AtCodecMarker for f64 {}
impl  AtCodecMarker for f32 {}
impl  AtCodecMarker for String {}
impl  AtCodecMarker for &'static str {}

impl<T: AtIntoValue> AtIntoParams for Vec<T> {
  fn into_params(self) -> Result<AtParams, BoxError> {
    let values = self
      .into_iter()
      .map(|i| i.into_value())
      .collect::<Result<Vec<_>, BoxError>>()?;

    Ok(AtParams::Positional(values))
  }
}

impl AtIntoValue for AtValue {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(self)
  }
}

// Unsigned integers
impl AtIntoValue for u8 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

impl AtIntoValue for u16 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

impl AtIntoValue for u32 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

impl AtIntoValue for u64 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

// Signed integers
impl AtIntoValue for i8 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

impl AtIntoValue for i16 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

impl AtIntoValue for i32 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self as i64))
  }
}

impl AtIntoValue for i64 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(self))
  }
}

// Floating point
impl AtIntoValue for f32 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Real(self as f64))
  }
}

impl AtIntoValue for f64 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Real(self))
  }
}

// Boolean
impl AtIntoValue for bool {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(if self { 1 } else { 0 }))
  }
}

// String types
impl AtIntoValue for String {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Text(self))
  }
}

impl AtIntoValue for &str {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Text(self.to_string()))
  }
}

// References to primitives
impl AtIntoValue for &u8 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &u16 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &u32 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &u64 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &i8 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &i16 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &i32 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self as i64))
  }
}

impl AtIntoValue for &i64 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(*self))
  }
}

impl AtIntoValue for &f32 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Real(*self as f64))
  }
}

impl AtIntoValue for &f64 {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Real(*self))
  }
}

impl AtIntoValue for &bool {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Integer(if *self { 1 } else { 0 }))
  }
}

impl AtIntoValue for &String {
  fn into_value(self) -> Result<AtValue, BoxError> {
    Ok(AtValue::Text(self.clone()))
  }
}