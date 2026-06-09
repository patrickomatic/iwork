use crate::Error;
use crate::protobuf::{ProtoMessage, read_varint};

const CHUNK_HEADER_LEN: usize = 4;
const SNAPPY_CHUNK_TYPE: u8 = 0;
/// Decompressed bytes per Snappy chunk emitted by [`IwaArchive::encode`],
/// matching the 64 KiB window real iWork writers use.
const IWA_CHUNK_SIZE: usize = 64 * 1024;

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

    pub fn leading_object_references(&self) -> Vec<u64> {
        let mut references = Vec::new();
        let mut cursor = 0;

        while cursor < self.body.len() {
            let Ok(tag) = read_varint(&self.body, &mut cursor) else {
                break;
            };
            let field_number = tag >> 3;
            let wire_type = tag & 0x07;
            if field_number != 1 || wire_type != 2 {
                break;
            }

            let Ok(len_varint) = read_varint(&self.body, &mut cursor) else {
                break;
            };
            let Ok(len) = usize::try_from(len_varint) else {
                break;
            };
            let Some(field_end) = cursor.checked_add(len) else {
                break;
            };
            let Some(value) = self.body.get(cursor..field_end) else {
                break;
            };
            cursor = field_end;

            let mut value_cursor = 0;
            let Ok(inner_tag) = read_varint(value, &mut value_cursor) else {
                break;
            };
            if inner_tag != 8 {
                break;
            }
            let Ok(object_id) = read_varint(value, &mut value_cursor) else {
                break;
            };
            if value_cursor != value.len() {
                break;
            }

            references.push(object_id);
        }

        references
    }

    pub fn leading_object_references_len(&self) -> usize {
        let mut cursor = 0;

        while cursor < self.body.len() {
            let entry_start = cursor;
            let Ok(tag) = read_varint(&self.body, &mut cursor) else {
                break;
            };
            let field_number = tag >> 3;
            let wire_type = tag & 0x07;
            if field_number != 1 || wire_type != 2 {
                return entry_start;
            }

            let Ok(len_varint) = read_varint(&self.body, &mut cursor) else {
                return entry_start;
            };
            let Ok(len) = usize::try_from(len_varint) else {
                return entry_start;
            };
            let Some(field_end) = cursor.checked_add(len) else {
                return entry_start;
            };
            let Some(value) = self.body.get(cursor..field_end) else {
                return entry_start;
            };
            cursor = field_end;

            let mut value_cursor = 0;
            let Ok(inner_tag) = read_varint(value, &mut value_cursor) else {
                return entry_start;
            };
            if inner_tag != 8 {
                return entry_start;
            }
            let Ok(_) = read_varint(value, &mut value_cursor) else {
                return entry_start;
            };
            if value_cursor != value.len() {
                return entry_start;
            }
        }

        cursor
    }

    pub fn ascii_strings(&self, min_len: usize) -> Vec<String> {
        let mut strings = Vec::new();
        let mut current = Vec::new();

        for byte in &self.body {
            if byte.is_ascii_graphic() || *byte == b' ' {
                current.push(*byte);
                continue;
            }

            if current.len() >= min_len {
                strings.push(String::from_utf8_lossy(&current).into_owned());
            }
            current.clear();
        }

        if current.len() >= min_len {
            strings.push(String::from_utf8_lossy(&current).into_owned());
        }

        strings
    }

    /// Serializes an archive from its header packet and body into the on-disk
    /// `.iwa` byte stream (length-prefixed packet + body, Snappy-framed).
    ///
    /// The decompressed stream is split into 64 KiB Snappy chunks, matching the
    /// chunk size real iWork writers emit so the output stays within the bounds
    /// other readers assume.
    pub fn encode(header: IwaPacket, body: Vec<u8>) -> Result<Vec<u8>, Error> {
        let mut archive_bytes = Vec::new();
        let IwaPacket {
            bytes: header_bytes,
            ..
        } = header;
        let header_len = u64::try_from(header_bytes.len())
            .map_err(|_| Error::InvalidIwa("packet length overflow"))?;
        push_varint(&mut archive_bytes, header_len);
        archive_bytes.extend_from_slice(&header_bytes);
        archive_bytes.extend(body);

        let mut out = Vec::new();
        for window in archive_bytes.chunks(IWA_CHUNK_SIZE) {
            let compressed = compress_snappy_literal(window)?;
            let compressed_len = u32::try_from(compressed.len())
                .map_err(|_| Error::InvalidIwa("chunk length overflow"))?;
            if compressed_len > 0x00ff_ffff {
                return Err(Error::InvalidIwa("chunk length overflow"));
            }

            out.reserve(CHUNK_HEADER_LEN + compressed.len());
            out.push(SNAPPY_CHUNK_TYPE);
            out.push((compressed_len & 0xff) as u8);
            out.push(((compressed_len >> 8) & 0xff) as u8);
            out.push(((compressed_len >> 16) & 0xff) as u8);
            out.extend_from_slice(&compressed);
        }
        Ok(out)
    }

    /// Re-serializes this archive losslessly, reproducing the same header packet
    /// and body bytes a reader will observe (the Snappy framing may differ).
    pub fn reencode(&self) -> Result<Vec<u8>, Error> {
        Self::encode(self.header.clone(), self.body.clone())
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
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { offset: 0, bytes }
    }

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
    /// Raw `MessageInfo.version` bytes (`f2`, e.g. `[1, 0, 5]`). Real archives
    /// always carry this; it must be reproduced or Numbers rejects the file.
    pub message_version: Option<Vec<u8>>,
    pub body_hint: Option<u64>,
    pub object_references: Vec<IwaObjectReference>,
}

impl IwaArchiveDescriptor {
    fn decode(message: &ProtoMessage) -> Result<Self, Error> {
        let root_object_id = message.field(1).and_then(|field| field.value.as_varint());
        let mut kind_hint = None;
        let mut message_version = None;
        let mut body_hint = None;
        let mut object_references = Vec::new();

        if let Some(info_field) = message.field(2)
            && let Some(info) = maybe_decode_message(&info_field.value)
        {
            kind_hint = info.field(1).and_then(|field| field.value.as_varint());
            message_version = info
                .field(2)
                .and_then(|field| field.value.as_bytes())
                .map(<[u8]>::to_vec);
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

        Ok(Self {
            root_object_id,
            kind_hint,
            message_version,
            body_hint,
            object_references,
        })
    }

    pub fn encode_message(&self) -> Result<ProtoMessage, Error> {
        let mut fields = Vec::new();

        if let Some(root_object_id) = self.root_object_id {
            fields.push(crate::protobuf::ProtoField::varint(1, root_object_id));
        }

        let mut info_fields = Vec::new();
        if let Some(kind_hint) = self.kind_hint {
            info_fields.push(crate::protobuf::ProtoField::varint(1, kind_hint));
        }
        if let Some(version) = &self.message_version {
            info_fields.push(crate::protobuf::ProtoField::bytes(2, version.clone()));
        }
        if let Some(body_hint) = self.body_hint {
            info_fields.push(crate::protobuf::ProtoField::varint(3, body_hint));
        }
        for object_reference in &self.object_references {
            let message = object_reference.encode_message()?;
            info_fields.push(crate::protobuf::ProtoField::message(4, &message)?);
        }
        if !info_fields.is_empty() {
            fields.push(crate::protobuf::ProtoField::message(
                2,
                &ProtoMessage::new(info_fields),
            )?);
        }

        Ok(ProtoMessage::new(fields))
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

impl IwaObjectReference {
    pub fn encode_message(&self) -> Result<ProtoMessage, Error> {
        let mut fields = Vec::new();
        if let Some(object_id) = self.object_id {
            fields.push(crate::protobuf::ProtoField::bytes(
                1,
                encode_varint_bytes(object_id),
            ));
        }
        if let Some(kind_hint) = self.kind_hint {
            fields.push(crate::protobuf::ProtoField::varint(2, kind_hint));
        }
        if let Some(state_hint) = self.state_hint {
            fields.push(crate::protobuf::ProtoField::varint(3, state_hint));
        }
        Ok(ProtoMessage::new(fields))
    }
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

fn compress_snappy_literal(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let len = u64::try_from(bytes.len())
        .map_err(|_| Error::InvalidIwa("snappy output length overflow"))?;
    push_varint(&mut out, len);

    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let chunk_len = (bytes.len() - cursor).min(60);
        out.push(
            u8::try_from((chunk_len - 1) << 2)
                .map_err(|_| Error::InvalidIwa("snappy literal tag overflow"))?,
        );
        out.extend_from_slice(&bytes[cursor..cursor + chunk_len]);
        cursor += chunk_len;
    }

    Ok(out)
}

fn encode_varint_bytes(value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    push_varint(&mut out, value);
    out
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
