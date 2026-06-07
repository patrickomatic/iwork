//! Narrow parser for `Metadata/Properties.plist`.
//!
//! Fixture documents have shown that this plist may be either XML or binary.
//! We only implement the subset needed to surface a stable group of metadata
//! keys from iWork packages:
//!
//! - `documentUUID`
//! - `fileFormatVersion`
//! - `isMultiPage`
//! - `revision`
//! - `stableDocumentUUID`
//! - `versionUUID`
//!
//! The XML parser accepts only `<string>`, `<true/>`, and `<false/>` values in
//! a top-level dictionary. The binary parser accepts only the object types that
//! appear in current fixtures: strings, booleans, and dictionaries.

use std::collections::BTreeMap;

use crate::Error;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertiesPlist {
    pub document_uuid: Option<String>,
    pub file_format_version: Option<String>,
    pub is_multi_page: Option<bool>,
    pub revision: Option<String>,
    pub stable_document_uuid: Option<String>,
    pub version_uuid: Option<String>,
}

/// Parses the `Metadata/Properties.plist` payload from an iWork package.
pub fn parse_properties_plist(bytes: &[u8]) -> Result<PropertiesPlist, Error> {
    if bytes.starts_with(b"bplist00") {
        return parse_binary_properties_plist(bytes);
    }

    let xml = std::str::from_utf8(bytes).map_err(|_| Error::InvalidPlist("expected UTF-8"))?;
    parse_xml_properties_plist(xml)
}

fn parse_xml_properties_plist(xml: &str) -> Result<PropertiesPlist, Error> {
    let mut bools = BTreeMap::new();
    let mut strings = BTreeMap::new();
    let dict_start = xml
        .find("<dict>")
        .ok_or(Error::InvalidPlist("missing <dict>"))?;
    let dict_end = xml
        .rfind("</dict>")
        .ok_or(Error::InvalidPlist("missing </dict>"))?;
    let mut rest = &xml[dict_start + "<dict>".len()..dict_end];

    while let Some(key_start) = rest.find("<key>") {
        rest = &rest[key_start + "<key>".len()..];
        let key_end = rest
            .find("</key>")
            .ok_or(Error::InvalidPlist("unterminated <key>"))?;
        let key = &rest[..key_end];
        rest = &rest[key_end + "</key>".len()..];
        let value = rest.trim_start();

        if let Some(stripped) = value.strip_prefix("<string>") {
            let value_end = stripped
                .find("</string>")
                .ok_or(Error::InvalidPlist("unterminated <string>"))?;
            strings.insert(key.to_owned(), stripped[..value_end].to_owned());
            rest = &stripped[value_end + "</string>".len()..];
            continue;
        }

        if let Some(stripped) = value.strip_prefix("<true/>") {
            bools.insert(key.to_owned(), true);
            rest = stripped;
            continue;
        }

        if let Some(stripped) = value.strip_prefix("<false/>") {
            bools.insert(key.to_owned(), false);
            rest = stripped;
            continue;
        }

        return Err(Error::InvalidPlist("unsupported value type"));
    }

    Ok(PropertiesPlist {
        document_uuid: strings.remove("documentUUID"),
        file_format_version: strings.remove("fileFormatVersion"),
        is_multi_page: bools.remove("isMultiPage"),
        revision: strings.remove("revision"),
        stable_document_uuid: strings.remove("stableDocumentUUID"),
        version_uuid: strings.remove("versionUUID"),
    })
}

fn parse_binary_properties_plist(bytes: &[u8]) -> Result<PropertiesPlist, Error> {
    if bytes.len() < 40 {
        return Err(Error::InvalidPlist("binary plist is too short"));
    }

    let trailer = &bytes[bytes.len() - 32..];
    let offset_int_size = trailer[6] as usize;
    let object_ref_size = trailer[7] as usize;
    let num_objects = u64_to_usize(read_be_u64(trailer, 8)?, "number of objects")?;
    let top_object = u64_to_usize(read_be_u64(trailer, 16)?, "top object index")?;
    let offset_table_offset = u64_to_usize(read_be_u64(trailer, 24)?, "offset table start")?;

    if offset_int_size == 0 || object_ref_size == 0 {
        return Err(Error::InvalidPlist("invalid trailer sizes"));
    }

    let offset_table_size = num_objects
        .checked_mul(offset_int_size)
        .ok_or(Error::InvalidPlist("offset table overflow"))?;
    let offset_table_end = offset_table_offset
        .checked_add(offset_table_size)
        .ok_or(Error::InvalidPlist("offset table overflow"))?;
    if offset_table_end > bytes.len() - 32 {
        return Err(Error::InvalidPlist("offset table out of bounds"));
    }

    let mut offsets = Vec::with_capacity(num_objects);
    for index in 0..num_objects {
        let start = offset_table_offset + index * offset_int_size;
        offsets.push(read_be_usize(bytes, start, offset_int_size)?);
    }

    let object = parse_binary_plist_object(bytes, &offsets, object_ref_size, top_object)?;
    let BinaryPlistObject::Dict(dict) = object else {
        return Err(Error::InvalidPlist("top object is not a dictionary"));
    };

    Ok(PropertiesPlist {
        document_uuid: dict_get_string(&dict, "documentUUID"),
        file_format_version: dict_get_string(&dict, "fileFormatVersion"),
        is_multi_page: dict_get_bool(&dict, "isMultiPage"),
        revision: dict_get_string(&dict, "revision"),
        stable_document_uuid: dict_get_string(&dict, "stableDocumentUUID"),
        version_uuid: dict_get_string(&dict, "versionUUID"),
    })
}

#[derive(Debug, Clone)]
enum BinaryPlistObject {
    String(String),
    Bool(bool),
    Dict(BTreeMap<String, BinaryPlistObject>),
}

fn dict_get_string(dict: &BTreeMap<String, BinaryPlistObject>, key: &str) -> Option<String> {
    match dict.get(key) {
        Some(BinaryPlistObject::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn dict_get_bool(dict: &BTreeMap<String, BinaryPlistObject>, key: &str) -> Option<bool> {
    match dict.get(key) {
        Some(BinaryPlistObject::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn parse_binary_plist_object(
    bytes: &[u8],
    offsets: &[usize],
    object_ref_size: usize,
    object_index: usize,
) -> Result<BinaryPlistObject, Error> {
    let offset = *offsets
        .get(object_index)
        .ok_or(Error::InvalidPlist("object index out of bounds"))?;
    let marker = *bytes
        .get(offset)
        .ok_or(Error::InvalidPlist("object offset out of bounds"))?;
    let object_type = marker >> 4;
    let object_info = (marker & 0x0F) as usize;

    match (object_type, object_info) {
        (0x0, 0x8) => Ok(BinaryPlistObject::Bool(false)),
        (0x0, 0x9) => Ok(BinaryPlistObject::Bool(true)),
        (0x5, _) => {
            let (len, data_start) = parse_plist_length(bytes, offset, object_info)?;
            let data_end = data_start
                .checked_add(len)
                .ok_or(Error::InvalidPlist("string length overflow"))?;
            let value = bytes
                .get(data_start..data_end)
                .ok_or(Error::InvalidPlist("ascii string out of bounds"))?;
            Ok(BinaryPlistObject::String(
                String::from_utf8(value.to_vec()).map_err(Error::InvalidUtf8)?,
            ))
        }
        (0x6, _) => {
            let (len, data_start) = parse_plist_length(bytes, offset, object_info)?;
            let byte_len = len
                .checked_mul(2)
                .ok_or(Error::InvalidPlist("utf16 string length overflow"))?;
            let data_end = data_start
                .checked_add(byte_len)
                .ok_or(Error::InvalidPlist("utf16 string length overflow"))?;
            let data = bytes
                .get(data_start..data_end)
                .ok_or(Error::InvalidPlist("utf16 string out of bounds"))?;
            let code_units = data
                .chunks_exact(2)
                .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>();
            let value = String::from_utf16(&code_units)
                .map_err(|_| Error::InvalidPlist("invalid utf16 string"))?;
            Ok(BinaryPlistObject::String(value))
        }
        (0xD, _) => {
            let (len, refs_start) = parse_plist_length(bytes, offset, object_info)?;
            let keys_start = refs_start;
            let values_start = keys_start
                .checked_add(
                    len.checked_mul(object_ref_size)
                        .ok_or(Error::InvalidPlist("dictionary reference overflow"))?,
                )
                .ok_or(Error::InvalidPlist("dictionary reference overflow"))?;

            let mut dict = BTreeMap::new();
            for index in 0..len {
                let key_ref =
                    read_be_usize(bytes, keys_start + index * object_ref_size, object_ref_size)?;
                let value_ref = read_be_usize(
                    bytes,
                    values_start + index * object_ref_size,
                    object_ref_size,
                )?;
                let key_object =
                    parse_binary_plist_object(bytes, offsets, object_ref_size, key_ref)?;
                let BinaryPlistObject::String(key) = key_object else {
                    return Err(Error::InvalidPlist("dictionary key is not a string"));
                };
                let value = parse_binary_plist_object(bytes, offsets, object_ref_size, value_ref)?;
                dict.insert(key, value);
            }

            Ok(BinaryPlistObject::Dict(dict))
        }
        _ => Err(Error::InvalidPlist("unsupported binary plist object")),
    }
}

fn parse_plist_length(
    bytes: &[u8],
    offset: usize,
    object_info: usize,
) -> Result<(usize, usize), Error> {
    if object_info < 0x0F {
        return Ok((object_info, offset + 1));
    }

    let int_marker = *bytes
        .get(offset + 1)
        .ok_or(Error::InvalidPlist("missing length integer"))?;
    if int_marker >> 4 != 0x1 {
        return Err(Error::InvalidPlist("length integer is not an int object"));
    }

    let int_power = (int_marker & 0x0F) as usize;
    let int_len = 1usize
        .checked_shl(
            u32::try_from(int_power)
                .map_err(|_| Error::InvalidPlist("length integer is too large"))?,
        )
        .ok_or(Error::InvalidPlist("length integer is too large"))?;
    let len_start = offset + 2;
    let len = read_be_usize(bytes, len_start, int_len)?;
    Ok((len, len_start + int_len))
}

fn read_be_usize(bytes: &[u8], offset: usize, width: usize) -> Result<usize, Error> {
    let slice = bytes
        .get(offset..offset + width)
        .ok_or(Error::InvalidPlist("integer out of bounds"))?;
    let mut value = 0usize;
    for byte in slice {
        value = value
            .checked_shl(8)
            .ok_or(Error::InvalidPlist("integer overflow"))?
            | (*byte as usize);
    }
    Ok(value)
}

fn read_be_u64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or(Error::InvalidPlist("u64 out of bounds"))?;
    Ok(u64::from_be_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn u64_to_usize(value: u64, context: &'static str) -> Result<usize, Error> {
    usize::try_from(value).map_err(|_| Error::InvalidPlist(context))
}
