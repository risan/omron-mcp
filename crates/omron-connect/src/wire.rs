//! Small serde helpers for OMRON's loosely typed JSON.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

/// A number that arrives as a JSON number or as a string. Anything that does
/// not parse becomes NaN, so one odd field never fails a whole sync.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Num(pub f64);

impl Num {
    pub fn to_i64(self) -> Option<i64> {
        if self.0.is_finite() {
            Some(self.0.round() as i64)
        } else {
            None
        }
    }

    pub fn is_nonzero(self) -> bool {
        self.0.is_finite() && self.0 != 0.0
    }
}

pub(crate) fn nonzero(value: Option<Num>) -> bool {
    value.is_some_and(Num::is_nonzero)
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NumVisitor;

        impl Visitor<'_> for NumVisitor {
            type Value = Num;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number or a numeric string")
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Num, E> {
                Ok(Num(value as f64))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Num, E> {
                Ok(Num(value as f64))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Num, E> {
                Ok(Num(value))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Num, E> {
                Ok(Num(value.trim().parse().unwrap_or(f64::NAN)))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Num, E> {
                Ok(Num(f64::from(u8::from(value))))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Num, E> {
                Ok(Num(f64::NAN))
            }
        }

        deserializer.deserialize_any(NumVisitor)
    }
}
