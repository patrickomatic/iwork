use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoMessage {
    fields: Vec<ProtoField>,
}

impl ProtoMessage {
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

    pub fn new(fields: Vec<ProtoField>) -> Self {
        Self { fields }
    }

    pub fn fields(&self) -> &[ProtoField] {
        &self.fields
    }

    pub fn field(&self, number: u32) -> Option<&ProtoField> {
        self.fields.iter().find(|field| field.number == number)
    }

    pub fn fields_by_number(&self, number: u32) -> impl Iterator<Item = &ProtoField> {
        self.fields
            .iter()
            .filter(move |field| field.number == number)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for field in &self.fields {
            field.encode_into(&mut out)?;
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoField {
    pub number: u32,
    pub value: ProtoValue,
}

impl ProtoField {
    pub fn varint(number: u32, value: u64) -> Self {
        Self {
            number,
            value: ProtoValue::Varint(value),
        }
    }

    pub fn fixed64(number: u32, value: u64) -> Self {
        Self {
            number,
            value: ProtoValue::Fixed64(value),
        }
    }

    pub fn bytes(number: u32, value: impl Into<Vec<u8>>) -> Self {
        Self {
            number,
            value: ProtoValue::LengthDelimited(value.into()),
        }
    }

    pub fn string(number: u32, value: impl Into<String>) -> Self {
        Self::bytes(number, value.into().into_bytes())
    }

    pub fn message(number: u32, value: &ProtoMessage) -> Result<Self, Error> {
        Ok(Self::bytes(number, value.encode()?))
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoValue {
    Varint(u64),
    Fixed64(u64),
    LengthDelimited(Vec<u8>),
    Fixed32(u32),
}

impl ProtoValue {
    pub fn as_varint(&self) -> Option<u64> {
        match self {
            Self::Varint(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::LengthDelimited(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_message(&self) -> Result<Option<ProtoMessage>, Error> {
        match self {
            Self::LengthDelimited(value) => Ok(Some(ProtoMessage::decode(value)?)),
            _ => Ok(None),
        }
    }

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
