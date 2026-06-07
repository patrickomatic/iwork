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

    pub fn fields(&self) -> &[ProtoField] {
        &self.fields
    }

    pub fn field(&self, number: u32) -> Option<&ProtoField> {
        self.fields.iter().find(|field| field.number == number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoField {
    pub number: u32,
    pub value: ProtoValue,
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
