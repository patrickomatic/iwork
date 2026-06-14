use crate::Error;
use crate::protobuf::{ProtoMessage, read_varint};

const CHUNK_HEADER_LEN: usize = 4;
const SNAPPY_CHUNK_TYPE: u8 = 0;
/// Decompressed bytes per Snappy chunk emitted by [`IwaArchive::encode`],
/// matching the 64 KiB window real iWork writers use.
const IWA_CHUNK_SIZE: usize = 64 * 1024;

/// A decoded `.iwa` archive: the Snappy-framed, protobuf-structured binary
/// files that make up the content of an iWork package.
///
/// On disk each `.iwa` is a stream of Snappy chunks (kind byte 0). After
/// decompression the bytes form an `ArchiveInfo` header packet followed by
/// zero or more `(ArchiveInfo, payload)` object records. This type decodes
/// that structure and exposes the header/descriptor, the raw body, and a
/// parsed object stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaArchive {
    chunks: Vec<IwaChunk>,
    header: IwaPacket,
    descriptor: IwaArchiveDescriptor,
    body: Vec<u8>,
}

impl IwaArchive {
    /// Decode a `.iwa` file from its raw on-disk bytes.
    ///
    /// Decompresses every Snappy chunk, then parses the decompressed stream
    /// into a header [`IwaPacket`], a root [`IwaArchiveDescriptor`], and a
    /// body that contains the rest of the archive's object records.
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

    /// The Snappy chunks that make up this archive's on-disk form.
    pub fn chunks(&self) -> &[IwaChunk] {
        &self.chunks
    }

    /// The leading `ArchiveInfo` packet (the archive header).
    pub fn header(&self) -> &IwaPacket {
        &self.header
    }

    /// The root object's descriptor, decoded from the archive header.
    pub fn descriptor(&self) -> &IwaArchiveDescriptor {
        &self.descriptor
    }

    /// The raw decompressed bytes that follow the header packet.
    ///
    /// Contains the payloads of all objects in the archive interleaved with
    /// their `ArchiveInfo` length-prefix records. Use [`Self::objects`] to
    /// get a parsed view.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Decodes every object stored in this archive.
    ///
    /// A `.iwa` archive is a stream of `(ArchiveInfo, payload)` records: the
    /// leading `ArchiveInfo` is carried in [`IwaArchive::header`] and its payload
    /// begins [`IwaArchive::body`]; any remaining objects follow as
    /// `varint(info_len) ArchiveInfo payload` records. Each `ArchiveInfo` shares
    /// the [`IwaArchiveDescriptor`] shape, so the same decoder names the object's
    /// type, version, and references.
    ///
    /// `ArchiveInfo.message_infos` (field 2) is **repeated**: an object's stored
    /// form can be several concatenated protobuf messages. The cursor therefore
    /// advances by the sum of every `MessageInfo.length`, while [`IwaObject::payload`]
    /// exposes only the first (primary) message — the one whose type identifies
    /// the object. Skipping only the first length would desynchronize the stream
    /// and silently drop every later object.
    ///
    /// The walk is tolerant: it stops at the first record it cannot decode and
    /// returns the objects recovered so far rather than failing.
    pub fn objects(&self) -> Vec<IwaObject> {
        let mut objects = Vec::new();

        // The leading object's ArchiveInfo is the header packet; its descriptor
        // is already decoded, but the header message also gives the full
        // multi-message payload span.
        let (first_len, total_len) = self
            .header
            .decode_message()
            .ok()
            .map_or((self.body.len(), self.body.len()), |message| {
                message_info_lengths(&message)
            });
        let first_len = first_len.min(self.body.len());
        objects.push(IwaObject {
            identifier: self.descriptor.root_object_id,
            message_type: self.descriptor.kind_hint,
            version: self.descriptor.message_version.clone(),
            references: self.descriptor.object_references.clone(),
            payload: self.body[..first_len].to_vec(),
        });

        let mut cursor = total_len.min(self.body.len());
        while cursor < self.body.len() {
            let Ok(info_len_varint) = read_varint(&self.body, &mut cursor) else {
                break;
            };
            let Ok(info_len) = usize::try_from(info_len_varint) else {
                break;
            };
            let Some(info_end) = cursor.checked_add(info_len) else {
                break;
            };
            let Some(info_bytes) = self.body.get(cursor..info_end) else {
                break;
            };
            cursor = info_end;

            let Ok(info_message) = ProtoMessage::decode(info_bytes) else {
                break;
            };
            let Ok(descriptor) = IwaArchiveDescriptor::decode(&info_message) else {
                break;
            };

            let (first_len, total_len) = message_info_lengths(&info_message);
            let Some(payload_end) = cursor.checked_add(first_len) else {
                break;
            };
            let Some(payload) = self.body.get(cursor..payload_end) else {
                break;
            };
            let Some(next_cursor) = cursor.checked_add(total_len) else {
                break;
            };
            if next_cursor > self.body.len() {
                break;
            }
            cursor = next_cursor;

            objects.push(IwaObject {
                identifier: descriptor.root_object_id,
                message_type: descriptor.kind_hint,
                version: descriptor.message_version,
                references: descriptor.object_references,
                payload: payload.to_vec(),
            });
        }

        objects
    }

    /// Reads the leading `field 1` object-reference varints from the body.
    ///
    /// The body of some `.iwa` archives begins with a sequence of `{field 1:
    /// {inner field 8: object_id}}` records before the first real object. These
    /// give a fast list of the object IDs the archive's root object directly
    /// references, without having to decode the full object stream.
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

    /// Byte length consumed by the leading object-reference block.
    ///
    /// Returns the number of bytes at the start of the body that belong to the
    /// leading reference sequence (see [`Self::leading_object_references`]).
    /// The first real object payload starts at this offset.
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

    /// Extracts printable ASCII runs from the raw body, for heuristic scanning.
    ///
    /// A run is a maximal sequence of graphic ASCII characters and spaces. Only
    /// runs at least `min_len` characters long are returned.
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

/// Metadata for one Snappy chunk within an `.iwa` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaChunk {
    /// The chunk kind byte (always `0` for Snappy in supported archives).
    pub kind: u8,
    /// Byte length of the compressed payload on disk.
    pub compressed_len: usize,
    /// Byte length after Snappy decompression.
    pub decompressed_len: usize,
}

/// A length-prefixed protobuf packet within an IWA archive stream.
///
/// An `IwaPacket` is either the leading header packet (whose content is an
/// `ArchiveInfo` / `IwaArchiveDescriptor`) or an intermediate info record
/// that precedes an object payload in the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaPacket {
    pub offset: usize,
    bytes: Vec<u8>,
}

impl IwaPacket {
    /// Wrap raw `ArchiveInfo` protobuf bytes into a packet for encoding.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { offset: 0, bytes }
    }

    /// The raw protobuf bytes of this packet.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Decode the packet bytes as a [`ProtoMessage`].
    pub fn decode_message(&self) -> Result<ProtoMessage, Error> {
        ProtoMessage::decode(&self.bytes)
    }
}

/// The decoded `ArchiveInfo` header that describes one object in an IWA
/// archive — its identity, message type, and cross-archive references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaArchiveDescriptor {
    /// The object identifier of this archive's root object.
    pub root_object_id: Option<u64>,
    /// The iWork message type of the root object (e.g. `6001` for `TableModel`).
    pub kind_hint: Option<u64>,
    /// Raw `MessageInfo.version` bytes (e.g. `[1, 0, 5]`). Real archives
    /// always carry this; it must be reproduced or Numbers rejects the file.
    pub message_version: Option<Vec<u8>>,
    /// Optional body length hint from the `MessageInfo` (field 3).
    pub body_hint: Option<u64>,
    /// Cross-object references declared by the root object's `ArchiveInfo`.
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

/// Returns `(first_message_len, total_payload_len)` for an `ArchiveInfo`.
///
/// `ArchiveInfo.message_infos` (field 2) is repeated; each `MessageInfo` carries
/// its serialized length in field 3. The first message is the object's primary
/// message; the total span (which the object-stream cursor must skip) is the sum
/// over all messages.
fn message_info_lengths(archive_info: &ProtoMessage) -> (usize, usize) {
    let mut first = 0usize;
    let mut total = 0usize;
    for (index, info_field) in archive_info.fields_by_number(2).enumerate() {
        let Some(length) = maybe_decode_message(&info_field.value)
            .and_then(|info| info.field(3).and_then(|field| field.value.as_varint()))
            .and_then(|length| usize::try_from(length).ok())
        else {
            continue;
        };
        if index == 0 {
            first = length;
        }
        total = total.saturating_add(length);
    }
    (first, total)
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

/// A cross-object reference declared in an `ArchiveInfo` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaObjectReference {
    /// The referenced object's identifier.
    pub object_id: Option<u64>,
    /// The message type of the referenced object.
    pub kind_hint: Option<u64>,
    /// An opaque state field from the reference record.
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

/// A single object decoded from an archive's `(ArchiveInfo, payload)` stream.
///
/// `message_type` is the Apple iWork message type identifier (the same value
/// reported as the archive descriptor's `kind_hint`); see
/// [`crate::numbers::message_type_name`] for the known mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IwaObject {
    pub identifier: Option<u64>,
    pub message_type: Option<u64>,
    /// Raw `MessageInfo.version` bytes, preserved for faithful re-encoding.
    pub version: Option<Vec<u8>>,
    pub references: Vec<IwaObjectReference>,
    pub payload: Vec<u8>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid IWA header packet (empty `ArchiveInfo` protobuf).
    /// An empty protobuf is legal: `IwaArchiveDescriptor::decode` treats every
    /// field as optional and returns all-`None` when they are absent.
    fn empty_header() -> IwaPacket {
        IwaPacket::new(vec![])
    }

    /// Build a header packet from a fully-populated descriptor.
    fn header_from(descriptor: &IwaArchiveDescriptor) -> IwaPacket {
        let msg = descriptor.encode_message().unwrap();
        IwaPacket::new(msg.encode().unwrap())
    }

    #[test]
    fn encode_decode_empty_body() {
        let encoded = IwaArchive::encode(empty_header(), vec![]).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        assert_eq!(decoded.body(), &[] as &[u8]);
        assert_eq!(decoded.chunks().len(), 1);
        assert_eq!(decoded.descriptor().root_object_id, None);
        assert_eq!(decoded.descriptor().kind_hint, None);
    }

    #[test]
    fn encode_decode_body_preserved() {
        let body = b"hello world".to_vec();
        let encoded = IwaArchive::encode(empty_header(), body.clone()).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        assert_eq!(decoded.body(), body.as_slice());
    }

    #[test]
    fn encode_decode_descriptor_preserved() {
        let descriptor = IwaArchiveDescriptor {
            root_object_id: Some(42),
            kind_hint: Some(6001),
            message_version: Some(vec![1, 0, 5]),
            body_hint: None,
            object_references: vec![IwaObjectReference {
                object_id: Some(99),
                kind_hint: Some(2),
                state_hint: None,
            }],
        };
        let encoded = IwaArchive::encode(header_from(&descriptor), vec![]).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        let d = decoded.descriptor();
        assert_eq!(d.root_object_id, Some(42));
        assert_eq!(d.kind_hint, Some(6001));
        assert_eq!(d.message_version, Some(vec![1, 0, 5]));
        assert_eq!(d.object_references.len(), 1);
        assert_eq!(d.object_references[0].object_id, Some(99));
        assert_eq!(d.object_references[0].kind_hint, Some(2));
        assert_eq!(d.object_references[0].state_hint, None);
    }

    #[test]
    fn reencode_roundtrip_preserves_body() {
        let body = b"test reencode body".to_vec();
        let encoded = IwaArchive::encode(empty_header(), body.clone()).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        let reencoded = decoded.reencode().unwrap();
        let redecoded = IwaArchive::decode(&reencoded).unwrap();
        assert_eq!(redecoded.body(), body.as_slice());
        assert_eq!(redecoded.descriptor().root_object_id, None);
    }

    #[test]
    fn decode_empty_bytes_fails() {
        let result = IwaArchive::decode(&[]);
        assert!(matches!(
            result,
            Err(Error::InvalidIwa("archive contained no chunks"))
        ));
    }

    #[test]
    fn decode_unsupported_chunk_type_fails() {
        // A 4-byte chunk header with kind=1 and compressed_len=0: no payload needed.
        let bytes = [1u8, 0, 0, 0];
        let result = IwaArchive::decode(&bytes);
        assert!(matches!(result, Err(Error::UnsupportedIwaChunkType(1))));
    }

    #[test]
    fn decode_truncated_chunk_header_fails() {
        // Only 3 bytes — header needs 4.
        let result = IwaArchive::decode(&[0u8, 0, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn small_archive_produces_single_chunk() {
        let body = vec![0x41u8; 100]; // 100 'A' bytes
        let encoded = IwaArchive::encode(empty_header(), body.clone()).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        assert_eq!(decoded.chunks().len(), 1);
        assert_eq!(decoded.body(), body.as_slice());
    }

    #[test]
    fn large_body_produces_multiple_chunks() {
        // Body larger than IWA_CHUNK_SIZE (64 KiB) forces multiple Snappy chunks.
        let body = vec![0x42u8; IWA_CHUNK_SIZE + 1];
        let encoded = IwaArchive::encode(empty_header(), body.clone()).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        assert!(decoded.chunks().len() >= 2);
        assert_eq!(decoded.body(), body.as_slice());
    }

    #[test]
    fn ascii_strings_extracts_printable_runs() {
        let mut body = b"hello".to_vec();
        body.push(0x00); // non-printable separator
        body.extend_from_slice(b"world");
        let encoded = IwaArchive::encode(empty_header(), body).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        let strings = decoded.ascii_strings(3);
        assert!(
            strings.iter().any(|s| s == "hello"),
            "expected 'hello' in {strings:?}"
        );
        assert!(
            strings.iter().any(|s| s == "world"),
            "expected 'world' in {strings:?}"
        );
    }

    #[test]
    fn ascii_strings_respects_min_len() {
        let mut body = b"ab".to_vec(); // length 2, below threshold
        body.push(0x00);
        body.extend_from_slice(b"long_enough");
        let encoded = IwaArchive::encode(empty_header(), body).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        let strings = decoded.ascii_strings(3);
        assert!(
            !strings.iter().any(|s| s == "ab"),
            "'ab' should be excluded"
        );
        assert!(strings.iter().any(|s| s == "long_enough"));
    }

    #[test]
    fn chunk_decompressed_len_matches_body() {
        // varint(0) = 1 byte header-len prefix + 0 header bytes + 4 body bytes = 5 total
        let body = b"test".to_vec();
        let encoded = IwaArchive::encode(empty_header(), body).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        let chunks = decoded.chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, 0);
        // 5 decompressed bytes: 1 (varint 0) + 0 (empty header) + 4 (body)
        assert_eq!(chunks[0].decompressed_len, 5);
    }

    #[test]
    fn leading_object_references_empty_for_empty_body() {
        let encoded = IwaArchive::encode(empty_header(), vec![]).unwrap();
        let decoded = IwaArchive::decode(&encoded).unwrap();
        assert!(decoded.leading_object_references().is_empty());
        assert_eq!(decoded.leading_object_references_len(), 0);
    }

    #[test]
    fn iwa_packet_decode_message_on_empty_is_ok() {
        let packet = empty_header();
        assert!(packet.decode_message().is_ok());
        assert_eq!(packet.decode_message().unwrap().fields().len(), 0);
    }
}
