use serde::de::{self, IntoDeserializer};

pub struct CoercingString(pub String);

macro_rules! impl_deserialize_num {
    ($method:ident, $visit:ident, $type:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: de::Visitor<'de>,
        {
            let val = self.0.parse::<$type>().map_err(de::Error::custom)?;
            visitor.$visit(val)
        }
    };
}

impl<'de> de::Deserializer<'de> for CoercingString {
    type Error = de::value::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_str(&self.0)
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        if self.0.eq_ignore_ascii_case("yes") || self.0.eq_ignore_ascii_case("true") {
            visitor.visit_bool(true)
        } else if self.0.eq_ignore_ascii_case("no") || self.0.eq_ignore_ascii_case("false") {
            visitor.visit_bool(false)
        } else {
            Err(de::Error::custom(format!("invalid boolean value: '{}'", self.0)))
        }
    }

    impl_deserialize_num!(deserialize_i8, visit_i8, i8);
    impl_deserialize_num!(deserialize_i16, visit_i16, i16);
    impl_deserialize_num!(deserialize_i32, visit_i32, i32);
    impl_deserialize_num!(deserialize_i64, visit_i64, i64);
    impl_deserialize_num!(deserialize_u8, visit_u8, u8);
    impl_deserialize_num!(deserialize_u16, visit_u16, u16);
    impl_deserialize_num!(deserialize_u32, visit_u32, u32);
    impl_deserialize_num!(deserialize_u64, visit_u64, u64);
    impl_deserialize_num!(deserialize_f32, visit_f32, f32);
    impl_deserialize_num!(deserialize_f64, visit_f64, f64);

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_str(&self.0)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_str(&self.0)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    serde::forward_to_deserialize_any! {
        i128 u128 char bytes byte_buf unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }

    fn is_human_readable(&self) -> bool {
        true
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_enum(EnumDeserializer { val: &self.0 })
    }
}

struct EnumDeserializer<'a> {
    val: &'a str,
}

impl<'de> de::EnumAccess<'de> for EnumDeserializer<'_> {
    type Error = de::value::Error;
    type Variant = Self;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(self.val.into_deserializer())?;
        Ok((variant, self))
    }
}

impl<'de> de::VariantAccess<'de> for EnumDeserializer<'_> {
    type Error = de::value::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T>(self, _seed: T) -> Result<T::Value, Self::Error>
    where
        T: de::DeserializeSeed<'de>,
    {
        Err(de::Error::custom(
            "Complex enums (Newtype variants) are not supported in configuration files",
        ))
    }

    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        Err(de::Error::custom(
            "Complex enums (Tuple variants) are not supported in configuration files",
        ))
    }

    fn struct_variant<V>(self, _fields: &'static [&'static str], _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        Err(de::Error::custom(
            "Complex enums (Struct variants) are not supported in configuration files",
        ))
    }
}

impl IntoDeserializer<'_, de::value::Error> for CoercingString {
    type Deserializer = Self;
    fn into_deserializer(self) -> Self::Deserializer {
        self
    }
}
