use crate::Error;
use crate::protobuf::{ProtoMessage, read_varint};

const CHUNK_HEADER_LEN: usize = 4;
const SNAPPY_CHUNK_TYPE: u8 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaArchive {
    chunks: Vec<IwaChunk>,
    header: IwaPacket,
    descriptor: IwaArchiveDescriptor,
    body: Vec<u8>,
}

impl IwaArchive {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut chunks = Vec::new();
        let mut archive_bytes = Vec::new();
        let mut cursor = 0;

        while cursor < bytes.len() {
            let header_end = cursor
                .checked_add(CHUNK_HEADER_LEN)
                .ok_or(Error::InvalidIwa("chunk header overflow"))?;
            let header = bytes
                .get(cursor..header_end)
                .ok_or(Error::Truncated("iwa chunk header"))?;
            let kind = header[0];
            let compressed_len = usize::from(header[1])
                | (usize::from(header[2]) << 8)
                | (usize::from(header[3]) << 16);
            cursor = header_end;

            let chunk_end = cursor
                .checked_add(compressed_len)
                .ok_or(Error::InvalidIwa("chunk length overflow"))?;
            let payload = bytes
                .get(cursor..chunk_end)
                .ok_or(Error::Truncated("iwa chunk payload"))?;
            cursor = chunk_end;

            if kind != SNAPPY_CHUNK_TYPE {
                return Err(Error::UnsupportedIwaChunkType(kind));
            }

            let decompressed = decompress_snappy(payload)?;
            archive_bytes.extend_from_slice(&decompressed);
            chunks.push(IwaChunk {
                kind,
                compressed_len,
                decompressed_len: decompressed.len(),
            });
        }

        if chunks.is_empty() {
            return Err(Error::InvalidIwa("archive contained no chunks"));
        }

        let (header, body) = decode_archive_stream(&archive_bytes)?;
        let descriptor = IwaArchiveDescriptor::decode(&header.decode_message()?)?;
        Ok(Self {
            chunks,
            header,
            descriptor,
            body,
        })
    }

    pub fn chunks(&self) -> &[IwaChunk] {
        &self.chunks
    }

    pub fn header(&self) -> &IwaPacket {
        &self.header
    }

    pub fn descriptor(&self) -> &IwaArchiveDescriptor {
        &self.descriptor
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaChunk {
    pub kind: u8,
    pub compressed_len: usize,
    pub decompressed_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaPacket {
    pub offset: usize,
    bytes: Vec<u8>,
}

impl IwaPacket {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn decode_message(&self) -> Result<ProtoMessage, Error> {
        ProtoMessage::decode(&self.bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaArchiveDescriptor {
    pub root_object_id: Option<u64>,
    pub kind_hint: Option<u64>,
    pub body_hint: Option<u64>,
    pub object_references: Vec<IwaObjectReference>,
}

impl IwaArchiveDescriptor {
    fn decode(message: &ProtoMessage) -> Result<Self, Error> {
        let root_object_id = message.field(1).and_then(|field| field.value.as_varint());
        let mut kind_hint = None;
        let mut body_hint = None;
        let mut object_references = Vec::new();

        if let Some(info_field) = message.field(2) {
            if let Some(info) = maybe_decode_message(&info_field.value) {
                kind_hint = info.field(1).and_then(|field| field.value.as_varint());
                body_hint = info.field(3).and_then(|field| field.value.as_varint());

                for object_field in info.fields_by_number(4) {
                    let Some(object_message) = maybe_decode_message(&object_field.value) else {
                        continue;
                    };

                    let object_id = object_message
                        .field(1)
                        .map(|field| decode_object_id_hint(&field.value))
                        .transpose()?
                        .flatten();
                    let kind_hint = object_message
                        .field(2)
                        .and_then(|field| field.value.as_varint());
                    let state_hint = object_message
                        .field(3)
                        .and_then(|field| field.value.as_varint());

                    object_references.push(IwaObjectReference {
                        object_id,
                        kind_hint,
                        state_hint,
                    });
                }
            }
        }

        Ok(Self {
            root_object_id,
            kind_hint,
            body_hint,
            object_references,
        })
    }
}

fn decode_object_id_hint(value: &crate::protobuf::ProtoValue) -> Result<Option<u64>, Error> {
    if let Some(object_id) = value.decode_varint_bytes()? {
        return Ok(Some(object_id));
    }

    let Some(message) = maybe_decode_message(value) else {
        return Ok(None);
    };

    if let Some(object_id) = message.field(1).and_then(|field| field.value.as_varint()) {
        return Ok(Some(object_id));
    }

    if let Some(field) = message.field(1) {
        return decode_object_id_hint(&field.value);
    }

    Ok(None)
}

fn maybe_decode_message(value: &crate::protobuf::ProtoValue) -> Option<ProtoMessage> {
    value
        .as_bytes()
        .and_then(|bytes| ProtoMessage::decode(bytes).ok())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaObjectReference {
    pub object_id: Option<u64>,
    pub kind_hint: Option<u64>,
    pub state_hint: Option<u64>,
}

fn decode_archive_stream(bytes: &[u8]) -> Result<(IwaPacket, Vec<u8>), Error> {
    let mut cursor = 0;
    let packet_len = usize::try_from(read_varint(bytes, &mut cursor)?)
        .map_err(|_| Error::InvalidIwa("packet length overflow"))?;
    let packet_end = cursor
        .checked_add(packet_len)
        .ok_or(Error::InvalidIwa("packet length overflow"))?;
    let packet_bytes = bytes
        .get(cursor..packet_end)
        .ok_or(Error::Truncated("iwa packet"))?;
    let header = IwaPacket {
        offset: 0,
        bytes: packet_bytes.to_vec(),
    };

    Ok((header, bytes[packet_end..].to_vec()))
}

fn decompress_snappy(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut cursor = 0;
    let expected_len = usize::try_from(read_varint(bytes, &mut cursor)?)
        .map_err(|_| Error::InvalidIwa("snappy output length overflow"))?;
    let mut out = Vec::with_capacity(expected_len);

    while cursor < bytes.len() {
        let tag = *bytes.get(cursor).ok_or(Error::Truncated("snappy tag"))?;
        cursor += 1;

        match tag & 0x03 {
            0 => {
                let len_code = usize::from(tag >> 2);
                let literal_len = if len_code < 60 {
                    len_code + 1
                } else {
                    let extra_bytes = len_code - 59;
                    let extra_end = cursor
                        .checked_add(extra_bytes)
                        .ok_or(Error::InvalidIwa("snappy literal length overflow"))?;
                    let extra = bytes
                        .get(cursor..extra_end)
                        .ok_or(Error::Truncated("snappy literal length"))?;
                    cursor = extra_end;

                    let mut len = 0usize;
                    for (index, byte) in extra.iter().enumerate() {
                        len |= usize::from(*byte) << (index * 8);
                    }
                    len + 1
                };

                let literal_end = cursor
                    .checked_add(literal_len)
                    .ok_or(Error::InvalidIwa("snappy literal overflow"))?;
                let literal = bytes
                    .get(cursor..literal_end)
                    .ok_or(Error::Truncated("snappy literal"))?;
                out.extend_from_slice(literal);
                cursor = literal_end;
            }
            1 => {
                let len = usize::from((tag >> 2) & 0x07) + 4;
                let low = usize::from(*bytes.get(cursor).ok_or(Error::Truncated("snappy copy"))?);
                cursor += 1;
                let high = usize::from(tag & 0xe0) << 3;
                copy_from_output(&mut out, high | low, len)?;
            }
            2 => {
                let len = usize::from(tag >> 2) + 1;
                let offset_end = cursor
                    .checked_add(2)
                    .ok_or(Error::InvalidIwa("snappy copy overflow"))?;
                let offset_bytes = bytes
                    .get(cursor..offset_end)
                    .ok_or(Error::Truncated("snappy copy offset"))?;
                cursor = offset_end;
                let offset = usize::from(u16::from_le_bytes([offset_bytes[0], offset_bytes[1]]));
                copy_from_output(&mut out, offset, len)?;
            }
            3 => {
                let len = usize::from(tag >> 2) + 1;
                let offset_end = cursor
                    .checked_add(4)
                    .ok_or(Error::InvalidIwa("snappy copy overflow"))?;
                let offset_bytes = bytes
                    .get(cursor..offset_end)
                    .ok_or(Error::Truncated("snappy copy offset"))?;
                cursor = offset_end;
                let offset = usize::try_from(u32::from_le_bytes([
                    offset_bytes[0],
                    offset_bytes[1],
                    offset_bytes[2],
                    offset_bytes[3],
                ]))
                .map_err(|_| Error::InvalidIwa("snappy copy offset overflow"))?;
                copy_from_output(&mut out, offset, len)?;
            }
            _ => return Err(Error::InvalidIwa("unsupported snappy tag")),
        }
    }

    if out.len() != expected_len {
        return Err(Error::InvalidIwa("snappy length mismatch"));
    }

    Ok(out)
}

fn copy_from_output(out: &mut Vec<u8>, offset: usize, len: usize) -> Result<(), Error> {
    if offset == 0 || offset > out.len() {
        return Err(Error::InvalidIwa("invalid snappy copy offset"));
    }

    let start = out.len() - offset;
    for index in 0..len {
        let byte = *out
            .get(start + index)
            .ok_or(Error::InvalidIwa("snappy copy out of bounds"))?;
        out.push(byte);
    }

    Ok(())
}
