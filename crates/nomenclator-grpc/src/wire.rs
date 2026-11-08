//! Converts between the library's serialized shapes and the protobuf contract, in both
//! directions, for every operation.
//!
//! The contract has the same fields as the library's serde output, so one walk over the
//! message descriptors converts any message:
//!
//! - An enum is converted to its `snake_case` name (`EXTRA_KIND_BEHIND_THE_SCENES` is
//!   `behind_the_scenes`). Value 0 (`..._UNSPECIFIED`, `..._UNSET`) is `null`.
//! - A `Span` is `[start, end]`.
//! - A oneof named `kind` is internally tagged: `{ kind: movie, ...the movie's fields }`.
//!   Any other oneof contains only its chosen field.
//! - A `...List` message with one repeated field is that list. The wrapper is used for a
//!   map value, or for a list whose absence differs from its emptiness.
//! - An unset optional or message field is `null`.
//!
//! A field or value that is missing from the contract is an error. It is never dropped.

use prost_reflect::{
    DescriptorPool, DynamicMessage, EnumDescriptor, FieldDescriptor, Kind, MapKey,
    MessageDescriptor, ReflectMessage, Value as WireValue,
};
use serde_json::{Map, Value as Json};
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

const SPAN_MESSAGE: &str = "nomenclator.v1.Span";
const INTERNALLY_TAGGED_ONEOF: &str = "kind";

static CONTRACT_DESCRIPTORS: LazyLock<DescriptorPool> = LazyLock::new(|| {
    DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/nomenclator_descriptors.bin")).as_slice(),
    )
    .expect("the compiled contract's descriptors decode")
});

/// An error for a value that the wire contract cannot represent, or a wire value outside
/// the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError(String);

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for WireError {}

/// Converts `message` to the library's serde shape. Fails if the message contains a
/// value outside the contract.
pub fn message_to_json<M: prost::Message + prost::Name>(message: &M) -> Result<Json, WireError> {
    let dynamic =
        DynamicMessage::decode(contract_message::<M>()?, message.encode_to_vec().as_slice())
            .map_err(|error| WireError(format!("{}: {error}", M::full_name())))?;
    dynamic_message_to_json(&dynamic)
}

/// Converts the library's serde shape to a message of type `M`. Fails if `json` contains
/// a field or value that the contract does not define.
pub fn json_to_message<M: prost::Message + prost::Name + Default>(
    json: &Json,
) -> Result<M, WireError> {
    json_to_dynamic_message(json, &contract_message::<M>()?, M::NAME)?
        .transcode_to()
        .map_err(|error| WireError(format!("{}: {error}", M::full_name())))
}

fn contract_message<M: prost::Name>() -> Result<MessageDescriptor, WireError> {
    CONTRACT_DESCRIPTORS
        .get_message_by_name(&M::full_name())
        .ok_or_else(|| {
            WireError(format!(
                "{}: the message is not in the wire contract",
                M::full_name()
            ))
        })
}

fn dynamic_message_to_json(message: &DynamicMessage) -> Result<Json, WireError> {
    let descriptor = message.descriptor();
    if descriptor.full_name() == SPAN_MESSAGE {
        let bound = |name| field_to_json(message, &named_field(&descriptor, name));
        return Ok(Json::Array(vec![bound("start")?, bound("end")?]));
    }
    if let Some(list_field) = list_wrapper_field(&descriptor) {
        return field_to_json(message, &list_field);
    }
    let mut object = Map::new();
    for field in descriptor.fields() {
        let Some(oneof) = field
            .containing_oneof()
            .filter(|oneof| !oneof.is_synthetic())
        else {
            object.insert(field.name().to_owned(), field_to_json(message, &field)?);
            continue;
        };
        if !message.has_field(&field) {
            continue;
        }
        let value = field_to_json(message, &field)?;
        match value {
            Json::Object(variant_fields) if oneof.name() == INTERNALLY_TAGGED_ONEOF => {
                object.insert(oneof.name().to_owned(), field.name().into());
                object.extend(variant_fields);
            }
            value => {
                object.insert(field.name().to_owned(), value);
            }
        }
    }
    Ok(Json::Object(object))
}

fn field_to_json(message: &DynamicMessage, field: &FieldDescriptor) -> Result<Json, WireError> {
    if field.supports_presence() && !message.has_field(field) {
        return Ok(Json::Null);
    }
    match &*message.get_field(field) {
        WireValue::List(items) => items
            .iter()
            .map(|item| single_value_to_json(item, &field.kind()))
            .collect::<Result<_, _>>()
            .map(Json::Array),
        WireValue::Map(entries) => {
            let value_kind = map_value_kind(field);
            let mut object = Map::new();
            for (key, value) in entries {
                let MapKey::String(key) = key else {
                    return Err(WireError(format!(
                        "{}: a map key is not a string",
                        field.name()
                    )));
                };
                object.insert(key.clone(), single_value_to_json(value, &value_kind)?);
            }
            Ok(Json::Object(object))
        }
        value => single_value_to_json(value, &field.kind()),
    }
}

fn single_value_to_json(value: &WireValue, kind: &Kind) -> Result<Json, WireError> {
    Ok(match (value, kind) {
        (WireValue::EnumNumber(number), Kind::Enum(enumeration)) => {
            enum_number_to_json(enumeration, *number)?
        }
        (WireValue::Message(message), _) => dynamic_message_to_json(message)?,
        (WireValue::Bool(value), _) => Json::Bool(*value),
        (WireValue::String(value), _) => Json::String(value.clone()),
        (WireValue::U32(value), _) => (*value).into(),
        (WireValue::U64(value), _) => (*value).into(),
        (WireValue::I32(value), _) => (*value).into(),
        (WireValue::I64(value), _) => (*value).into(),
        (value, _) => {
            return Err(WireError(format!(
                "the wire value {value:?} has an unsupported type"
            )));
        }
    })
}

fn json_to_dynamic_message(
    json: &Json,
    descriptor: &MessageDescriptor,
    at: &str,
) -> Result<DynamicMessage, WireError> {
    let mut message = DynamicMessage::new(descriptor.clone());
    if descriptor.full_name() == SPAN_MESSAGE {
        let [start, end] = json
            .as_array()
            .and_then(|bounds| <&[Json; 2]>::try_from(bounds.as_slice()).ok())
            .ok_or_else(|| WireError(format!("{at}: the span is not a [start, end] pair")))?;
        for (name, bound) in [("start", start), ("end", end)] {
            let field = named_field(descriptor, name);
            message.set_field(&field, json_to_single_value(bound, &field.kind(), at)?);
        }
        return Ok(message);
    }
    if let Some(list_field) = list_wrapper_field(descriptor) {
        message.set_field(&list_field, json_to_field_value(json, &list_field, at)?);
        return Ok(message);
    }
    let object = json
        .as_object()
        .ok_or_else(|| WireError(format!("{at}: the value {json} is not an object")))?;
    let tagged_oneof = descriptor
        .oneofs()
        .find(|oneof| !oneof.is_synthetic() && oneof.name() == INTERNALLY_TAGGED_ONEOF);
    if let Some(oneof) = tagged_oneof {
        let tag = object.get(INTERNALLY_TAGGED_ONEOF).and_then(Json::as_str);
        let variant = oneof
            .fields()
            .find(|field| Some(field.name()) == tag)
            .ok_or_else(|| WireError(format!("{at}: the kind {tag:?} is unknown")))?;
        let mut variant_fields = object.clone();
        variant_fields.remove(INTERNALLY_TAGGED_ONEOF);
        let variant_json = Json::Object(variant_fields);
        let value = json_to_field_value(&variant_json, &variant, &format!("{at}.{tag:?}"))?;
        message.set_field(&variant, value);
        return Ok(message);
    }
    for (name, value) in object {
        let field_at = format!("{at}.{name}");
        let field = descriptor.get_field_by_name(name).ok_or_else(|| {
            WireError(format!("{field_at}: the field is not in the wire contract"))
        })?;
        if !value.is_null() {
            message.set_field(&field, json_to_field_value(value, &field, &field_at)?);
        }
    }
    Ok(message)
}

fn json_to_field_value(
    json: &Json,
    field: &FieldDescriptor,
    at: &str,
) -> Result<WireValue, WireError> {
    if field.is_map() {
        let value_kind = map_value_kind(field);
        let object = json
            .as_object()
            .ok_or_else(|| WireError(format!("{at}: the value {json} is not a map")))?;
        let entries = object
            .iter()
            .map(|(key, value)| {
                let value = json_to_single_value(value, &value_kind, &format!("{at}.{key}"))?;
                Ok((MapKey::String(key.clone()), value))
            })
            .collect::<Result<HashMap<_, _>, WireError>>()?;
        return Ok(WireValue::Map(entries));
    }
    if field.is_list() {
        let items = json
            .as_array()
            .ok_or_else(|| WireError(format!("{at}: the value {json} is not a list")))?;
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                json_to_single_value(item, &field.kind(), &format!("{at}[{index}]"))
            })
            .collect::<Result<_, _>>()
            .map(WireValue::List);
    }
    json_to_single_value(json, &field.kind(), at)
}

fn json_to_single_value(json: &Json, kind: &Kind, at: &str) -> Result<WireValue, WireError> {
    let mismatch = || {
        WireError(format!(
            "{at}: the value {json} does not fit the type {kind:?}"
        ))
    };
    let unsigned = || json.as_u64().ok_or_else(mismatch);
    let signed = || json.as_i64().ok_or_else(mismatch);
    Ok(match kind {
        Kind::Message(descriptor) => {
            WireValue::Message(json_to_dynamic_message(json, descriptor, at)?)
        }
        Kind::Enum(enumeration) => {
            WireValue::EnumNumber(enum_number_from_json(enumeration, json).ok_or_else(mismatch)?)
        }
        Kind::String => WireValue::String(json.as_str().ok_or_else(mismatch)?.to_owned()),
        Kind::Bool => WireValue::Bool(json.as_bool().ok_or_else(mismatch)?),
        Kind::Uint32 => WireValue::U32(u32::try_from(unsigned()?).map_err(|_| mismatch())?),
        Kind::Uint64 => WireValue::U64(unsigned()?),
        Kind::Int32 => WireValue::I32(i32::try_from(signed()?).map_err(|_| mismatch())?),
        Kind::Int64 => WireValue::I64(signed()?),
        _ => return Err(mismatch()),
    })
}

/// Returns the prefix that every value of the enum shares, read from its value 0. For
/// `RULE_UNSPECIFIED`, it returns `RULE_`.
fn enum_value_prefix(enumeration: &EnumDescriptor) -> String {
    let none_value = enumeration.default_value();
    let none_name = none_value.name();
    let prefix = none_name
        .strip_suffix("UNSPECIFIED")
        .or_else(|| none_name.strip_suffix("UNSET"))
        .expect("an enum's value 0 is `..._UNSPECIFIED` or `..._UNSET`");
    prefix.to_owned()
}

fn enum_number_to_json(enumeration: &EnumDescriptor, number: i32) -> Result<Json, WireError> {
    if number == 0 {
        return Ok(Json::Null);
    }
    let value = enumeration.get_value(number).ok_or_else(|| {
        WireError(format!(
            "{number} is not a value of the enum {}",
            enumeration.name()
        ))
    })?;
    let prefix = enum_value_prefix(enumeration);
    let name = value.name().strip_prefix(&prefix).unwrap_or(value.name());
    Ok(Json::String(name.to_ascii_lowercase()))
}

fn enum_number_from_json(enumeration: &EnumDescriptor, json: &Json) -> Option<i32> {
    let name = json.as_str()?;
    let wire_name = format!(
        "{}{}",
        enum_value_prefix(enumeration),
        name.to_ascii_uppercase()
    );
    enumeration
        .get_value_by_name(&wire_name)
        .map(|value| value.number())
}

/// Returns the repeated field of a `...List` message, which represents that list. Returns
/// `None` if the message is not such a wrapper.
fn list_wrapper_field(descriptor: &MessageDescriptor) -> Option<FieldDescriptor> {
    if !descriptor.name().ends_with("List") {
        return None;
    }
    let mut fields = descriptor.fields();
    match (fields.next(), fields.next()) {
        (Some(field), None) if field.is_list() => Some(field),
        _ => None,
    }
}

fn map_value_kind(map_field: &FieldDescriptor) -> Kind {
    match map_field.kind() {
        Kind::Message(entry) => entry.map_entry_value_field().kind(),
        other => other,
    }
}

fn named_field(descriptor: &MessageDescriptor, name: &str) -> FieldDescriptor {
    descriptor
        .get_field_by_name(name)
        .unwrap_or_else(|| panic!("{} has a `{name}` field", descriptor.full_name()))
}
