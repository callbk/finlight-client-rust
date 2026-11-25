use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A key with three states: absent, `null` or a value.
///
/// Used for `content` and `sentiment`, whose presence depends on your plan and on `include`:
///
/// - [`Field::Absent`]: the key was not in the JSON (not entitled, not requested, or raw).
/// - [`Field::Null`]: the key was there with `null` (entitled and requested, but no value).
/// - [`Field::Value`]: the key carried a value.
///
/// When written back with serde, an absent field stays absent and `null` stays `null`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Field<T> {
    /// The key was not in the JSON.
    #[default]
    Absent,
    /// The key was in the JSON with `null`.
    Null,
    /// The key carried a value.
    Value(T),
}

impl<T> Field<T> {
    /// `true` when the key was in the JSON, `null` included.
    pub fn is_present(&self) -> bool {
        !matches!(self, Field::Absent)
    }

    /// `true` when the key was not in the JSON.
    pub fn is_absent(&self) -> bool {
        matches!(self, Field::Absent)
    }

    /// `true` when the key was in the JSON with `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Field::Null)
    }

    /// The value, or `None` when absent or `null`.
    pub fn value(&self) -> Option<&T> {
        match self {
            Field::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Converts into the value, or `None` when absent or `null`.
    pub fn into_value(self) -> Option<T> {
        match self {
            Field::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Borrows the content.
    pub fn as_ref(&self) -> Field<&T> {
        match self {
            Field::Absent => Field::Absent,
            Field::Null => Field::Null,
            Field::Value(v) => Field::Value(v),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Only called when the key is present; a missing key uses `Default` (Absent).
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(v) => Field::Value(v),
            None => Field::Null,
        })
    }
}

impl<T: Serialize> Serialize for Field<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // Fields that use Field skip serializing when absent; this arm is a fallback.
            Field::Absent | Field::Null => serializer.serialize_none(),
            Field::Value(v) => v.serialize(serializer),
        }
    }
}
