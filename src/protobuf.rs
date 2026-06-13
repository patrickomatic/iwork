use crate::Error;

/// A decoded protobuf message: an ordered list of fields.
///
/// Mirrors the protobuf wire format — fields are not de-duplicated; a field
/// number may appear more than once (repeated fields). Use [`Self::field`] for
/// the first occurrence and [`Self::fields_by_number`] to iterate all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoMessage {
    fields: Vec<ProtoField>,
}

impl ProtoMessage {
    /// Decode a raw protobuf message from `bytes`.
    ///
    /// Returns an error if any tag, varint, or length-delimited boundary is
    /// malformed or truncated. Wire types 3 and 4 (start/end group) are not
    /// supported and return an error.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut fields = Vec::new();
        let mut cursor = 0;

        while cursor < bytes.len() {
            let tag = read_varint(bytes, &mut cursor)?;
            if tag == 0 {
                return Err(Error::InvalidIwa("protobuf tag cannot be zero"));
            }

            let number = u32::try_from(tag >> 3)
                .map_err(|_| Error::InvalidIwa("protobuf field number overflow"))?;
            let wire_type = u8::try_from(tag & 0x07)
                .map_err(|_| Error::InvalidIwa("protobuf wire type overflow"))?;
            let value = match wire_type {
                0 => ProtoValue::Varint(read_varint(bytes, &mut cursor)?),
                1 => ProtoValue::Fixed64(read_fixed64(bytes, &mut cursor)?),
                2 => {
                    let len = usize::try_from(read_varint(bytes, &mut cursor)?)
                        .map_err(|_| Error::InvalidIwa("protobuf length overflow"))?;
                    let end = cursor
                        .checked_add(len)
                        .ok_or(Error::InvalidIwa("protobuf length overflow"))?;
                    let value = bytes
                        .get(cursor..end)
                        .ok_or(Error::Truncated("protobuf length-delimited field"))?;
                    cursor = end;
                    ProtoValue::LengthDelimited(value.to_vec())
                }
                5 => ProtoValue::Fixed32(read_fixed32(bytes, &mut cursor)?),
                _ => return Err(Error::InvalidIwa("unsupported protobuf wire type")),
            };

            fields.push(ProtoField { number, value });
        }

        Ok(Self { fields })
    }

    /// Construct a message from a pre-built list of fields, for encoding.
    pub fn new(fields: Vec<ProtoField>) -> Self {
        Self { fields }
    }

    /// All fields in wire order.
    pub fn fields(&self) -> &[ProtoField] {
        &self.fields
    }

    /// The first field with the given field number, or `None`.
    pub fn field(&self, number: u32) -> Option<&ProtoField> {
        self.fields.iter().find(|field| field.number == number)
    }

    /// All fields with the given field number, in wire order.
    pub fn fields_by_number(&self, number: u32) -> impl Iterator<Item = &ProtoField> {
        self.fields
            .iter()
            .filter(move |field| field.number == number)
    }

    /// Encode this message back to the protobuf wire format.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for field in &self.fields {
            field.encode_into(&mut out)?;
        }
        Ok(out)
    }
}

/// A single protobuf field: a number and its wire-typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoField {
    pub number: u32,
    pub value: ProtoValue,
}

impl ProtoField {
    /// Construct a wire-type-0 (varint) field.
    pub fn varint(number: u32, value: u64) -> Self {
        Self {
            number,
            value: ProtoValue::Varint(value),
        }
    }

    /// Construct a wire-type-1 (64-bit) field.
    pub fn fixed64(number: u32, value: u64) -> Self {
        Self {
            number,
            value: ProtoValue::Fixed64(value),
        }
    }

    /// Construct a wire-type-2 (length-delimited) field from raw bytes.
    pub fn bytes(number: u32, value: impl Into<Vec<u8>>) -> Self {
        Self {
            number,
            value: ProtoValue::LengthDelimited(value.into()),
        }
    }

    /// Construct a wire-type-2 field from a UTF-8 string (encoded as bytes).
    pub fn string(number: u32, value: impl Into<String>) -> Self {
        Self::bytes(number, value.into().into_bytes())
    }

    /// Construct a wire-type-2 field by encoding a nested message.
    pub fn message(number: u32, value: &ProtoMessage) -> Result<Self, Error> {
        Ok(Self::bytes(number, value.encode()?))
    }

    /// Construct a wire-type-5 (32-bit) field.
    pub fn fixed32(number: u32, value: u32) -> Self {
        Self {
            number,
            value: ProtoValue::Fixed32(value),
        }
    }

    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        let wire_type = self.value.wire_type();
        push_varint(out, u64::from((self.number << 3) | u32::from(wire_type)));
        self.value.encode_into(out)
    }
}

/// The wire-typed payload of a single protobuf field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoValue {
    Varint(u64),
    Fixed64(u64),
    LengthDelimited(Vec<u8>),
    Fixed32(u32),
}

impl ProtoValue {
    /// Return the varint value, or `None` if this is a different wire type.
    pub fn as_varint(&self) -> Option<u64> {
        match self {
            Self::Varint(value) => Some(*value),
            _ => None,
        }
    }

    /// Return the raw bytes of a length-delimited field, or `None`.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::LengthDelimited(value) => Some(value),
            _ => None,
        }
    }

    /// Decode a length-delimited field as a nested protobuf message, or return
    /// `None` if this is a different wire type.
    pub fn as_message(&self) -> Result<Option<ProtoMessage>, Error> {
        match self {
            Self::LengthDelimited(value) => Ok(Some(ProtoMessage::decode(value)?)),
            _ => Ok(None),
        }
    }

    /// Decode a length-delimited field that contains exactly one varint.
    ///
    /// Returns `Some(value)` when the bytes contain exactly one complete varint
    /// and no trailing bytes; `None` for any other wire type or byte layout.
    pub fn decode_varint_bytes(&self) -> Result<Option<u64>, Error> {
        match self {
            Self::LengthDelimited(value) => {
                let mut cursor = 0;
                let decoded = read_varint(value, &mut cursor)?;
                if cursor == value.len() {
                    Ok(Some(decoded))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn wire_type(&self) -> u8 {
        match self {
            Self::Varint(_) => 0,
            Self::Fixed64(_) => 1,
            Self::LengthDelimited(_) => 2,
            Self::Fixed32(_) => 5,
        }
    }

    fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        match self {
            Self::Varint(value) => {
                push_varint(out, *value);
            }
            Self::Fixed64(value) => out.extend_from_slice(&value.to_le_bytes()),
            Self::LengthDelimited(value) => {
                let len = u64::try_from(value.len())
                    .map_err(|_| Error::InvalidIwa("protobuf length overflow"))?;
                push_varint(out, len);
                out.extend_from_slice(value);
            }
            Self::Fixed32(value) => out.extend_from_slice(&value.to_le_bytes()),
        }
        Ok(())
    }
}

/// Read one varint from `bytes` starting at `*cursor`, advancing `*cursor` past it.
///
/// Returns an error if the varint is truncated or if the accumulated value
/// would require more than 64 bits (more than 9 continuation bytes).
pub fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let mut shift = 0u32;
    let mut value = 0u64;

    loop {
        if shift >= 64 {
            return Err(Error::InvalidIwa("varint overflow"));
        }

        let byte = *bytes.get(*cursor).ok_or(Error::Truncated("varint"))?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }

        shift += 7;
    }
}

fn read_fixed32(bytes: &[u8], cursor: &mut usize) -> Result<u32, Error> {
    let end = cursor
        .checked_add(4)
        .ok_or(Error::InvalidIwa("fixed32 overflow"))?;
    let slice = bytes.get(*cursor..end).ok_or(Error::Truncated("fixed32"))?;
    *cursor = end;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_fixed64(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let end = cursor
        .checked_add(8)
        .ok_or(Error::InvalidIwa("fixed64 overflow"))?;
    let slice = bytes.get(*cursor..end).ok_or(Error::Truncated("fixed64"))?;
    *cursor = end;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_single_varint_field() {
        // Proto spec example: field 1, varint 150 → [0x08, 0x96, 0x01]
        let msg = ProtoMessage::decode(&[0x08, 0x96, 0x01]).unwrap();
        assert_eq!(msg.fields().len(), 1);
        assert_eq!(msg.field(1).unwrap().value.as_varint(), Some(150));
    }

    #[test]
    fn decode_length_delimited_field() {
        // field 2, wire type 2, length 5, bytes "hello"
        let bytes = [0x12, 0x05, b'h', b'e', b'l', b'l', b'o'];
        let msg = ProtoMessage::decode(&bytes).unwrap();
        assert_eq!(msg.field(2).unwrap().value.as_bytes(), Some(b"hello" as &[u8]));
    }

    #[test]
    fn decode_fixed32_field() {
        // field 1, wire type 5, value 0x01020304 little-endian
        let bytes = [0x0d, 0x04, 0x03, 0x02, 0x01];
        let msg = ProtoMessage::decode(&bytes).unwrap();
        assert_eq!(msg.field(1).unwrap().value, ProtoValue::Fixed32(0x0102_0304));
    }

    #[test]
    fn decode_fixed64_field() {
        // field 1, wire type 1, value 0x0102030405060708 LE
        let bytes = [0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];
        let msg = ProtoMessage::decode(&bytes).unwrap();
        assert_eq!(
            msg.field(1).unwrap().value,
            ProtoValue::Fixed64(0x0102_0304_0506_0708)
        );
    }

    #[test]
    fn decode_multiple_fields_in_order() {
        // field 1 varint 1, field 2 string "hi"
        let bytes = [0x08, 0x01, 0x12, 0x02, b'h', b'i'];
        let msg = ProtoMessage::decode(&bytes).unwrap();
        assert_eq!(msg.fields().len(), 2);
        assert_eq!(msg.field(1).unwrap().value.as_varint(), Some(1));
        assert_eq!(msg.field(2).unwrap().value.as_bytes(), Some(b"hi" as &[u8]));
    }

    #[test]
    fn decode_empty_message() {
        let msg = ProtoMessage::decode(&[]).unwrap();
        assert!(msg.fields().is_empty());
    }

    #[test]
    fn field_returns_none_for_absent_number() {
        let msg = ProtoMessage::decode(&[0x08, 0x01]).unwrap();
        assert!(msg.field(99).is_none());
    }

    #[test]
    fn fields_by_number_returns_all_repeated_occurrences() {
        // field 1 varint appears twice (repeated field pattern)
        let bytes = [0x08, 0x01, 0x08, 0x02];
        let msg = ProtoMessage::decode(&bytes).unwrap();
        let vals: Vec<u64> = msg
            .fields_by_number(1)
            .filter_map(|f| f.value.as_varint())
            .collect();
        assert_eq!(vals, [1, 2]);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = ProtoMessage::new(vec![
            ProtoField::varint(1, 42),
            ProtoField::string(2, "hello"),
            ProtoField::fixed32(3, 0xDEAD_BEEF),
            ProtoField::fixed64(4, u64::MAX),
        ]);
        let encoded = original.encode().unwrap();
        let decoded = ProtoMessage::decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn nested_message_roundtrip() {
        let inner = ProtoMessage::new(vec![ProtoField::varint(1, 7)]);
        let outer = ProtoMessage::new(vec![ProtoField::message(2, &inner).unwrap()]);
        let bytes = outer.encode().unwrap();
        let decoded = ProtoMessage::decode(&bytes).unwrap();
        let inner_decoded = decoded
            .field(2)
            .unwrap()
            .value
            .as_message()
            .unwrap()
            .unwrap();
        assert_eq!(inner_decoded.field(1).unwrap().value.as_varint(), Some(7));
    }

    #[test]
    fn decode_rejects_tag_zero() {
        assert!(ProtoMessage::decode(&[0x00]).is_err());
    }

    #[test]
    fn decode_rejects_unsupported_wire_type() {
        // tag = field 1, wire type 3 → 0x0B
        assert!(ProtoMessage::decode(&[0x0B]).is_err());
    }

    #[test]
    fn decode_rejects_truncated_length_delimited() {
        // field 1, wire type 2, length 10, but only 3 payload bytes follow
        let bytes = [0x0a, 0x0a, 0x01, 0x02, 0x03];
        assert!(ProtoMessage::decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_truncated_varint() {
        // field 1, wire type 0, then a varint with continuation bit set but no next byte
        let bytes = [0x08, 0x80];
        assert!(ProtoMessage::decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_varint_overflow() {
        // tag 0x08 = field 1 varint, then 10 bytes all with continuation bit → shift overflow
        let mut bytes = vec![0x08u8];
        bytes.extend(std::iter::repeat(0x80u8).take(10));
        assert!(ProtoMessage::decode(&bytes).is_err());
    }

    #[test]
    fn decode_varint_bytes_extracts_single_varint() {
        // bytes containing varint 150 = [0x96, 0x01]
        let field = ProtoField::bytes(1, vec![0x96, 0x01]);
        assert_eq!(field.value.decode_varint_bytes().unwrap(), Some(150));
    }

    #[test]
    fn decode_varint_bytes_returns_none_for_wrong_wire_type() {
        let field = ProtoField::varint(1, 42);
        assert_eq!(field.value.decode_varint_bytes().unwrap(), None);
    }

    #[test]
    fn decode_varint_bytes_returns_none_when_trailing_bytes_remain() {
        // A valid varint followed by extra bytes → should return None (not a single-varint field)
        let field = ProtoField::bytes(1, vec![0x01, 0xFF]);
        assert_eq!(field.value.decode_varint_bytes().unwrap(), None);
    }
}
