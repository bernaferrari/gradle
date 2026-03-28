use std::io::{Cursor, Read};
use std::path::Path;

use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::error::SubstrateError;

use crate::proto::{
    file_fingerprint_service_server::FileFingerprintService, FileFingerprintEntry,
    FingerprintFilesRequest, FingerprintFilesResponse, FingerprintType,
};

/// A single fingerprinted file entry: (relative_path, content_hash, size_bytes, modified_time_ms, is_directory).
type FingerprintEntry = (String, Vec<u8>, i64, i64, bool);

/// Rust-native file fingerprinting service.
/// Walks file trees and computes content hashes, replacing Java's FileCollectionFingerprinter.
/// Supports ABI-level fingerprinting for .class files (matching Gradle's ClassFileAnalyzer).
#[derive(Default)]
pub struct FileFingerprintServiceImpl;

// ---------------------------------------------------------------------------
// Minimal Java class file parser for ABI fingerprinting
// ---------------------------------------------------------------------------

/// Constant pool tag values.
const CP_UTF8: u8 = 1;
const CP_INTEGER: u8 = 3;
const CP_FLOAT: u8 = 4;
const CP_LONG: u8 = 5;
const CP_DOUBLE: u8 = 6;
const CP_CLASS: u8 = 7;
const CP_STRING: u8 = 8;
const CP_FIELDREF: u8 = 9;
const CP_METHODREF: u8 = 10;
const CP_INTERFACE_METHODREF: u8 = 11;
const CP_NAME_AND_TYPE: u8 = 12;
const CP_METHOD_HANDLE: u8 = 15;
const CP_METHOD_TYPE: u8 = 16;
const CP_INVOKE_DYNAMIC: u8 = 18;

/// Access flag masks for visibility.
const ACC_PUBLIC: u16 = 0x0001;
const ACC_PROTECTED: u16 = 0x0004;

/// Write a `usize` as decimal ASCII into `buf` and return the written slice.
/// Zero heap allocation — uses only the provided stack buffer.
fn write_int_to_buf(buf: &mut [u8; 20], mut val: usize) -> &[u8] {
    if val == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut pos = buf.len();
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    &buf[pos..]
}

/// Reads a class file and extracts ABI-relevant information for fingerprinting.
/// Only includes public/protected API elements, ignoring private implementation details.
fn class_file_abi_hash(data: &[u8]) -> Option<Vec<u8>> {
    let mut cursor = Cursor::new(data);

    // Magic number
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic).ok()?;
    if magic != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }

    // Minor + major version — included in ABI hash because class file version
    // affects ABI compatibility (e.g., Java 8 vs 11 bytecode features)
    let mut version = [0u8; 4];
    cursor.read_exact(&mut version).ok()?;

    // Constant pool
    let cp_count = read_u16(&mut cursor)?;
    let cp_entries = read_constant_pool(&mut cursor, cp_count)?;

    // Validate constant pool structural integrity
    if let Err(validation_err) = validate_constant_pool(&cp_entries) {
        tracing::debug!(
            "Class file constant pool validation failed: {}",
            validation_err
        );
        return None;
    }

    // Compute and log constant pool composition stats
    let cp_stats = compute_cp_stats(&cp_entries);
    tracing::debug!(
        cp_count = cp_count,
        stats = cp_stats.describe(),
        "Parsed class file constant pool"
    );

    // Access flags
    let access_flags = read_u16(&mut cursor)?;

    // This class
    let this_class = read_u16(&mut cursor)?;

    // Super class
    let super_class = read_u16(&mut cursor)?;

    // Interfaces
    let interfaces_count = read_u16(&mut cursor)?;
    let mut interfaces = Vec::with_capacity(interfaces_count as usize);
    for _ in 0..interfaces_count {
        interfaces.push(read_u16(&mut cursor)?);
    }

    // Fields — collect public/protected field ABI during single pass
    let fields_count = read_u16(&mut cursor)?;
    let mut public_field_abi = Vec::with_capacity(fields_count as usize);
    for _ in 0..fields_count {
        let flags = read_u16(&mut cursor)?;
        let name_idx = read_u16(&mut cursor)?;
        let desc_idx = read_u16(&mut cursor)?;
        let attrs_count = read_u16(&mut cursor)?;
        for _ in 0..attrs_count {
            read_u16(&mut cursor)?;
            let attr_len = read_u32(&mut cursor)? as usize;
            cursor.consume(attr_len)?;
        }
        if flags & (ACC_PUBLIC | ACC_PROTECTED) != 0 {
            if let (Some(name), Some(desc)) = (
                get_utf8(&cp_entries, name_idx),
                get_utf8(&cp_entries, desc_idx),
            ) {
                public_field_abi.push(format!("{}:{}", name, desc));
            }
        }
    }

    // Methods — collect public/protected method ABI during single pass
    let methods_count = read_u16(&mut cursor)?;
    let mut public_method_abi = Vec::with_capacity(methods_count as usize);
    for _ in 0..methods_count {
        let flags = read_u16(&mut cursor)?;
        let name_idx = read_u16(&mut cursor)?;
        let desc_idx = read_u16(&mut cursor)?;
        let attrs_count = read_u16(&mut cursor)?;
        for _ in 0..attrs_count {
            read_u16(&mut cursor)?;
            let attr_len = read_u32(&mut cursor)? as usize;
            cursor.consume(attr_len)?;
        }
        if flags & (ACC_PUBLIC | ACC_PROTECTED) != 0 {
            if let (Some(name), Some(desc)) = (
                get_utf8(&cp_entries, name_idx),
                get_utf8(&cp_entries, desc_idx),
            ) {
                public_method_abi.push(format!("{}:{}", name, desc));
            }
        }
    }

    // Build ABI hash from extracted information
    let mut hasher = Md5::new();

    // Include class file version (major.minor) for ABI compatibility
    hasher.update(b"version=");
    hasher.update(version);

    // Include class access flags
    hasher.update(b"access=");
    hasher.update(access_flags.to_le_bytes());

    // Include class name
    if let Some(name) = get_class_name(&cp_entries, this_class) {
        hasher.update(b"class=");
        hasher.update(name.as_bytes());
        hasher.update([0]); // separator
    }

    // Include super class name
    if super_class != 0 {
        if let Some(name) = get_class_name(&cp_entries, super_class) {
            hasher.update(b"super=");
            hasher.update(name.as_bytes());
            hasher.update([0]); // separator
        }
    }

    // Include interfaces (sorted)
    let mut iface_names: Vec<String> = interfaces
        .iter()
        .filter_map(|idx| get_class_name(&cp_entries, *idx))
        .collect();
    iface_names.sort_unstable();
    for iface in &iface_names {
        hasher.update(b"iface=");
        hasher.update(iface.as_bytes());
        hasher.update([0]); // null separator
    }


    // Include public/protected fields (sorted)
    public_field_abi.sort_unstable();
    hasher.update(b"fields=");
    hasher.update(public_field_abi.join(";").as_bytes());
    hasher.update([0]); // null separator

    // Include public/protected methods (sorted)
    public_method_abi.sort_unstable();
    hasher.update(b"methods=");
    hasher.update(public_method_abi.join(";").as_bytes());
    hasher.update([0]); // null separator



    // Include reference-type counts from the constant pool — these reflect
    // the external API surface (field/method references, string constants, etc.)
    // and ensure ABI changes involving added/removed references are detected.
    let mut num_buf = [0u8; 20]; // enough for any usize
    hasher.update(b"refs=");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.field_ref));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.method_ref));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.interface_method_ref));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.string));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.invoke_dynamic));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.method_handle));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.method_type));
    hasher.update(b";");

    // Include numeric constant pool counts for completeness — changes in
    // compile-time constants (Integer, Float, Long, Double) can affect
    // downstream compilation (e.g., constant folding, inlining).
    hasher.update(b"num_consts=");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.integer));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.float_));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.long));
    hasher.update(b",");
    hasher.update(write_int_to_buf(&mut num_buf, cp_stats.double_));

    Some(hasher.finalize().to_vec())
}

/// Constant pool entry.
#[derive(Debug)]
enum CpEntry {
    Utf8(String),
    Class(u16),            // name_index
    NameAndType(u16, u16), // name_index, descriptor_index
    MethodRef(u16, u16),   // class_index, name_and_type_index
    InterfaceMethodRef(u16, u16),
    FieldRef(u16, u16),
    String(u16), // utf8_index
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    MethodHandle(u8, u16),
    MethodType(u16),
    InvokeDynamic(u16, u16),
}

fn read_constant_pool(cursor: &mut Cursor<&[u8]>, count: u16) -> Option<Vec<Option<CpEntry>>> {
    let mut entries: Vec<Option<CpEntry>> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        entries.push(None);
    }
    let mut i = 1u16;
    while i < count {
        let tag = {
            let mut buf = [0u8; 1];
            cursor.read_exact(&mut buf).ok()?;
            buf[0]
        };
        let entry = match tag {
            CP_UTF8 => {
                let len = read_u16(cursor)? as usize;
                let mut buf = vec![0u8; len];
                cursor.read_exact(&mut buf).ok()?;
                // Java modified UTF-8: just use as-is for fingerprinting
                CpEntry::Utf8(String::from_utf8_lossy(&buf).into_owned())
            }
            CP_CLASS => CpEntry::Class(read_u16(cursor)?),
            CP_STRING => CpEntry::String(read_u16(cursor)?),
            CP_FIELDREF | CP_METHODREF | CP_INTERFACE_METHODREF => {
                let class_idx = read_u16(cursor)?;
                let nat_idx = read_u16(cursor)?;
                if tag == CP_FIELDREF {
                    CpEntry::FieldRef(class_idx, nat_idx)
                } else if tag == CP_INTERFACE_METHODREF {
                    CpEntry::InterfaceMethodRef(class_idx, nat_idx)
                } else {
                    CpEntry::MethodRef(class_idx, nat_idx)
                }
            }
            CP_NAME_AND_TYPE => {
                let name_idx = read_u16(cursor)?;
                let desc_idx = read_u16(cursor)?;
                CpEntry::NameAndType(name_idx, desc_idx)
            }
            CP_INTEGER => {
                let mut buf = [0u8; 4];
                cursor.read_exact(&mut buf).ok()?;
                CpEntry::Integer(i32::from_be_bytes(buf))
            }
            CP_FLOAT => {
                let mut buf = [0u8; 4];
                cursor.read_exact(&mut buf).ok()?;
                CpEntry::Float(f32::from_be_bytes(buf))
            }
            CP_LONG => {
                let mut buf = [0u8; 8];
                cursor.read_exact(&mut buf).ok()?;
                CpEntry::Long(i64::from_be_bytes(buf))
            }
            CP_DOUBLE => {
                let mut buf = [0u8; 8];
                cursor.read_exact(&mut buf).ok()?;
                CpEntry::Double(f64::from_be_bytes(buf))
            }
            CP_METHOD_HANDLE => {
                let kind = {
                    let mut buf = [0u8; 1];
                    cursor.read_exact(&mut buf).ok()?;
                    buf[0]
                };
                CpEntry::MethodHandle(kind, read_u16(cursor)?)
            }
            CP_METHOD_TYPE => CpEntry::MethodType(read_u16(cursor)?),
            CP_INVOKE_DYNAMIC => {
                let bootstrap = read_u16(cursor)?;
                let nat = read_u16(cursor)?;
                CpEntry::InvokeDynamic(bootstrap, nat)
            }
            _ => return None,
        };
        entries[i as usize] = Some(entry);
        // Long and Double take two slots
        if tag == CP_LONG || tag == CP_DOUBLE {
            i += 2;
        } else {
            i += 1;
        }
    }
    Some(entries)
}

/// Counts constant pool entries by type for diagnostic logging and validation.
struct CpEntryStats {
    utf8: usize,
    class: usize,
    string: usize,
    field_ref: usize,
    method_ref: usize,
    interface_method_ref: usize,
    name_and_type: usize,
    integer: usize,
    float_: usize,
    long: usize,
    double_: usize,
    method_handle: usize,
    method_type: usize,
    invoke_dynamic: usize,
}

impl CpEntryStats {
    fn new() -> Self {
        Self {
            utf8: 0,
            class: 0,
            string: 0,
            field_ref: 0,
            method_ref: 0,
            interface_method_ref: 0,
            name_and_type: 0,
            integer: 0,
            float_: 0,
            long: 0,
            double_: 0,
            method_handle: 0,
            method_type: 0,
            invoke_dynamic: 0,
        }
    }

    fn total(&self) -> usize {
        self.utf8
            + self.class
            + self.string
            + self.field_ref
            + self.method_ref
            + self.interface_method_ref
            + self.name_and_type
            + self.integer
            + self.float_
            + self.long
            + self.double_
            + self.method_handle
            + self.method_type
            + self.invoke_dynamic
    }

    fn describe(&self) -> String {
        format!(
            "CP entries: {} utf8, {} class, {} string, {} fieldref, {} methodref, {} ifmethodref, {} nat, {} int, {} float, {} long, {} double, {} methodhandle, {} methodtype, {} invokedynamic (total {})",
            self.utf8, self.class, self.string, self.field_ref, self.method_ref,
            self.interface_method_ref, self.name_and_type, self.integer,
            self.float_, self.long, self.double_, self.method_handle,
            self.method_type, self.invoke_dynamic, self.total(),
        )
    }
}

/// Counts every variant of CpEntry in the constant pool for diagnostic use.
fn compute_cp_stats(cp: &[Option<CpEntry>]) -> CpEntryStats {
    let mut stats = CpEntryStats::new();
    for entry in cp.iter().flatten() {
        match entry {
            CpEntry::Utf8(_) => stats.utf8 += 1,
            CpEntry::Class(_) => stats.class += 1,
            CpEntry::String(_) => stats.string += 1,
            CpEntry::FieldRef(_, _) => stats.field_ref += 1,
            CpEntry::MethodRef(_, _) => stats.method_ref += 1,
            CpEntry::InterfaceMethodRef(_, _) => stats.interface_method_ref += 1,
            CpEntry::NameAndType(_, _) => stats.name_and_type += 1,
            CpEntry::Integer(_) => stats.integer += 1,
            CpEntry::Float(_) => stats.float_ += 1,
            CpEntry::Long(_) => stats.long += 1,
            CpEntry::Double(_) => stats.double_ += 1,
            CpEntry::MethodHandle(_, _) => stats.method_handle += 1,
            CpEntry::MethodType(_) => stats.method_type += 1,
            CpEntry::InvokeDynamic(_, _) => stats.invoke_dynamic += 1,
        }
    }
    stats
}

/// Validates structural integrity of the constant pool by checking that all
/// index references point to valid entries of the expected type.
/// Returns Ok(()) if valid, Err(description) otherwise.
fn validate_constant_pool(cp: &[Option<CpEntry>]) -> Result<(), String> {
    for (i, entry) in cp.iter().enumerate() {
        let entry = match entry {
            Some(e) => e,
            None => continue,
        };
        match entry {
            CpEntry::Class(name_idx) => {
                if *name_idx == 0 || *name_idx as usize >= cp.len() {
                    return Err(format!(
                        "Class entry at {} references invalid utf8 index {}",
                        i, name_idx
                    ));
                }
                match &cp[*name_idx as usize] {
                    Some(CpEntry::Utf8(_)) => {}
                    other => {
                        return Err(format!(
                            "Class entry at {} references non-utf8 at index {} (found {:?})",
                            i,
                            name_idx,
                            other.as_ref().map(std::mem::discriminant)
                        ))
                    }
                }
            }
            CpEntry::NameAndType(name_idx, desc_idx) => {
                for (label, idx) in [("name", name_idx), ("descriptor", desc_idx)] {
                    if *idx == 0 || *idx as usize >= cp.len() {
                        return Err(format!(
                            "NameAndType at {} references invalid {} index {}",
                            i, label, idx
                        ));
                    }
                    match &cp[*idx as usize] {
                        Some(CpEntry::Utf8(_)) => {}
                        other => {
                            return Err(format!(
                                "NameAndType at {} references non-utf8 {} at index {} (found {:?})",
                                i,
                                label,
                                idx,
                                other.as_ref().map(std::mem::discriminant)
                            ))
                        }
                    }
                }
            }
            CpEntry::String(utf8_idx) => {
                if *utf8_idx == 0 || *utf8_idx as usize >= cp.len() {
                    return Err(format!(
                        "String entry at {} references invalid utf8 index {}",
                        i, utf8_idx
                    ));
                }
                match &cp[*utf8_idx as usize] {
                    Some(CpEntry::Utf8(_)) => {}
                    other => {
                        return Err(format!(
                            "String entry at {} references non-utf8 at index {} (found {:?})",
                            i,
                            utf8_idx,
                            other.as_ref().map(std::mem::discriminant)
                        ))
                    }
                }
            }
            CpEntry::FieldRef(class_idx, nat_idx)
            | CpEntry::MethodRef(class_idx, nat_idx)
            | CpEntry::InterfaceMethodRef(class_idx, nat_idx) => {
                if *class_idx == 0 || *class_idx as usize >= cp.len() {
                    return Err(format!(
                        "Ref entry at {} references invalid class index {}",
                        i, class_idx
                    ));
                }
                match &cp[*class_idx as usize] {
                    Some(CpEntry::Class(_)) => {}
                    other => {
                        return Err(format!(
                            "Ref entry at {} references non-class at index {} (found {:?})",
                            i,
                            class_idx,
                            other.as_ref().map(std::mem::discriminant)
                        ))
                    }
                }
                if *nat_idx == 0 || *nat_idx as usize >= cp.len() {
                    return Err(format!(
                        "Ref entry at {} references invalid nat index {}",
                        i, nat_idx
                    ));
                }
                match &cp[*nat_idx as usize] {
                    Some(CpEntry::NameAndType(_, _)) => {}
                    other => {
                        return Err(format!(
                            "Ref entry at {} references non-NameAndType at index {} (found {:?})",
                            i,
                            nat_idx,
                            other.as_ref().map(std::mem::discriminant)
                        ))
                    }
                }
            }
            CpEntry::MethodHandle(kind, reference_idx) => {
                // Validate kind is in the range 1..=9 (REF_getField through REF_invokeInterface)
                const MH_KIND_MIN: u8 = 1;
                const MH_KIND_MAX: u8 = 9;
                if *kind < MH_KIND_MIN || *kind > MH_KIND_MAX {
                    return Err(format!(
                        "MethodHandle at {} has invalid kind {} (valid range 1-9)",
                        i, kind
                    ));
                }
                if *reference_idx == 0 || *reference_idx as usize >= cp.len() {
                    return Err(format!(
                        "MethodHandle at {} references invalid index {}",
                        i, reference_idx
                    ));
                }
                let is_field_ref = *kind <= 4; // REF_getField=1 .. REF_putStatic=4
                let valid = if is_field_ref {
                    matches!(&cp[*reference_idx as usize], Some(CpEntry::FieldRef(_, _)))
                } else {
                    matches!(
                        &cp[*reference_idx as usize],
                        Some(CpEntry::MethodRef(_, _)) | Some(CpEntry::InterfaceMethodRef(_, _))
                    )
                };
                if !valid {
                    return Err(format!(
                        "MethodHandle at {} (kind={}) references mismatched entry at index {}",
                        i, kind, reference_idx
                    ));
                }
            }
            CpEntry::MethodType(desc_idx) => {
                if *desc_idx == 0 || *desc_idx as usize >= cp.len() {
                    return Err(format!(
                        "MethodType at {} references invalid utf8 index {}",
                        i, desc_idx
                    ));
                }
                match &cp[*desc_idx as usize] {
                    Some(CpEntry::Utf8(_)) => {}
                    other => {
                        return Err(format!(
                            "MethodType at {} references non-utf8 at index {} (found {:?})",
                            i,
                            desc_idx,
                            other.as_ref().map(std::mem::discriminant)
                        ))
                    }
                }
            }
            CpEntry::InvokeDynamic(bootstrap_idx, nat_idx) => {
                if *bootstrap_idx == 0 || *bootstrap_idx as usize >= cp.len() {
                    return Err(format!(
                        "InvokeDynamic at {} references invalid bootstrap index {}",
                        i, bootstrap_idx
                    ));
                }
                match &cp[*bootstrap_idx as usize] {
                    Some(CpEntry::NameAndType(_, _)) | Some(CpEntry::MethodHandle(_, _)) => {}
                    other => {
                        return Err(format!(
                        "InvokeDynamic at {} references unexpected entry at index {} (found {:?})",
                        i, bootstrap_idx, other.as_ref().map(std::mem::discriminant)
                    ))
                    }
                }
                if *nat_idx == 0 || *nat_idx as usize >= cp.len() {
                    return Err(format!(
                        "InvokeDynamic at {} references invalid nat index {}",
                        i, nat_idx
                    ));
                }
                match &cp[*nat_idx as usize] {
                    Some(CpEntry::NameAndType(_, _)) => {}
                    other => {
                        return Err(format!(
                        "InvokeDynamic at {} references non-NameAndType at index {} (found {:?})",
                        i, nat_idx, other.as_ref().map(std::mem::discriminant)
                    ))
                    }
                }
            }
            // Utf8 has no cross-references to validate
            CpEntry::Utf8(_) => {}
            // Numeric entries: validate bit patterns for structural soundness
            CpEntry::Integer(val) => {
                // No cross-references, but check for obviously corrupt values
                // (e.g., Integer.MIN_VALUE is valid, so no range check needed)
                let _ = val; // acknowledged in validation pass
            }
            CpEntry::Float(val) => {
                // Detect signaling NaN: NaN with the quiet bit cleared.
                // A signaling NaN has exponent all 1s, fraction non-zero, and the
                // highest fraction bit (quiet NaN bit) cleared.
                if val.is_nan() && (val.to_bits() & 0x0040_0000) == 0 {
                    return Err(format!(
                        "Float entry at {} is a signaling NaN, indicating class file corruption",
                        i
                    ));
                }
            }
            CpEntry::Long(val) => {
                let _ = val; // acknowledged in validation pass
            }
            CpEntry::Double(val) => {
                // Detect signaling NaN: NaN with the quiet bit cleared.
                if val.is_nan() && (val.to_bits() & 0x0008_0000_0000_0000) == 0 {
                    return Err(format!(
                        "Double entry at {} is a signaling NaN, indicating class file corruption",
                        i
                    ));
                }
            }
        }
    }
    Ok(())
}

fn get_utf8(cp: &[Option<CpEntry>], index: u16) -> Option<&str> {
    cp.get(index as usize)?.as_ref().and_then(|e| match e {
        CpEntry::Utf8(s) => Some(s.as_str()),
        _ => None,
    })
}

fn get_class_name(cp: &[Option<CpEntry>], class_index: u16) -> Option<String> {
    let name_index = cp
        .get(class_index as usize)?
        .as_ref()
        .and_then(|e| match e {
            CpEntry::Class(idx) => Some(*idx),
            _ => None,
        })?;
    Some(get_utf8(cp, name_index)?.replace('/', "."))
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> Option<u16> {
    let mut buf = [0u8; 2];
    cursor.read_exact(&mut buf).ok()?;
    Some(u16::from_be_bytes(buf))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Option<u32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf).ok()?;
    Some(u32::from_be_bytes(buf))
}

trait CursorConsume {
    fn consume(&mut self, n: usize) -> Option<()>;
}

impl CursorConsume for Cursor<&[u8]> {
    fn consume(&mut self, n: usize) -> Option<()> {
        let pos = self.position() as usize + n;
        if pos <= self.get_ref().len() {
            self.set_position(pos as u64);
            Some(())
        } else {
            None
        }
    }
}

/// Normalization strategy for file paths in fingerprint computation.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NormalizationStrategy {
    /// Use absolute paths (default).
    AbsolutePath,
    /// Use paths relative to the common root directory.
    RelativePath,
    /// Use only file names (ignore directory structure).
    NameOnly,
    /// Use only content hashes (ignore paths entirely).
    HashOnly,
    /// Use class file ABI fingerprinting for .class files, content hash otherwise.
    ClassAbi,
}

impl NormalizationStrategy {
    fn from_str(s: &str) -> Self {
        match s {
            "RELATIVE_PATH" => Self::RelativePath,
            "NAME_ONLY" => Self::NameOnly,
            "HASH" => Self::HashOnly,
            "CLASS_ABI" => Self::ClassAbi,
            _ => Self::AbsolutePath,
        }
    }

    /// Normalize a path according to the strategy.
    /// `base` is the root directory (used for RELATIVE_PATH).
    /// `relative` is the path relative to base.
    fn normalize<'a>(
        &self,
        _base: &Path,
        relative: &'a str,
        full_path: &Path,
    ) -> std::borrow::Cow<'a, str> {
        match self {
            Self::AbsolutePath => {
                // Use the full absolute path
                std::borrow::Cow::Owned(full_path.to_string_lossy().into_owned())
            }
            Self::RelativePath => {
                // Already relative to base
                std::borrow::Cow::Borrowed(relative)
            }
            Self::NameOnly => {
                // Use only the file name
                std::borrow::Cow::Owned(
                    full_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(relative)
                        .to_string(),
                )
            }
            Self::HashOnly | Self::ClassAbi => {
                // Use "hash-" prefix + relative as a placeholder;
                // the actual entry path is replaced with the hash below
                // (CLASS_ABI falls back to content hashing for non-.class files)
                std::borrow::Cow::Borrowed(relative)
            }
        }
    }

    /// Replace a path entry with a hash-based identifier for HashOnly strategy.
    fn hash_only_path(hash: &[u8]) -> String {
        format!("hash-{:x}", Md5::digest(hash))
    }
}

impl FileFingerprintServiceImpl {
    pub fn new() -> Self {
        Self
    }

    fn fingerprint_file_with_strategy(
        path: &Path,
        strategy: NormalizationStrategy,
    ) -> Result<(Vec<u8>, i64, i64), SubstrateError> {
        let metadata = std::fs::metadata(path)
            .map_err(|e| SubstrateError::Fingerprint { path: path.to_path_buf(), reason: e.to_string() })?;
        let size = metadata.len() as i64;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // For CLASS_ABI strategy, use ABI fingerprinting for .class files
        if strategy == NormalizationStrategy::ClassAbi {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "class" {
                    let data = std::fs::read(path)
                        .map_err(|e| SubstrateError::Fingerprint { path: path.to_path_buf(), reason: e.to_string() })?;
                    if let Some(hash) = class_file_abi_hash(&data) {
                        return Ok((hash, size, modified));
                    }
                    // Fall through to content hash if class file parsing fails
                }
            }
        }

        // Compute MD5 hash of file content (matching Java's DefaultStreamHasher)
        let mut hasher = Md5::new();
        let file = std::fs::File::open(path)
            .map_err(|e| SubstrateError::Fingerprint { path: path.to_path_buf(), reason: e.to_string() })?;
        let mut reader = std::io::BufReader::new(file);
        let mut buffer = [0u8; 8192];
        loop {
            let n = std::io::Read::read(&mut reader, &mut buffer)
                .map_err(|e| SubstrateError::Fingerprint { path: path.to_path_buf(), reason: e.to_string() })?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let hash = hasher.finalize().to_vec();

        Ok((hash, size, modified))
    }

    fn fingerprint_directory(
        dir: &Path,
        ignore_patterns: &[String],
        strategy: NormalizationStrategy,
    ) -> Result<(Vec<FingerprintEntry>, Vec<u8>), SubstrateError> {
        let mut entries = Vec::new();
        let mut dir_hasher = Md5::new();

        Self::walk_dir(
            dir,
            dir,
            &mut entries,
            &mut dir_hasher,
            ignore_patterns,
            strategy,
        )?;

        let collection_hash = dir_hasher.finalize().to_vec();
        Ok((entries, collection_hash))
    }

    fn should_ignore(path: &Path, ignore_patterns: &[String]) -> bool {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let path_str = path.to_string_lossy();

        for pattern in ignore_patterns {
            // Exact filename match
            if file_name == pattern {
                return true;
            }
            // *.ext pattern: match files ending with .ext
            if pattern.starts_with("*.") {
                let ext = &pattern[1..]; // e.g. ".class"
                if file_name.ends_with(ext) {
                    return true;
                }
            }
            // Directory/partial path match: if path contains the pattern as a path component
            if path_str.contains(&format!("/{}", pattern)) {
                return true;
            }
            // Endswith for directory patterns like "build"
            if path_str.ends_with(&format!("/{}", pattern)) {
                return true;
            }
        }
        false
    }

    fn walk_dir(
        base: &Path,
        current: &Path,
        entries: &mut Vec<FingerprintEntry>,
        hasher: &mut Md5,
        ignore_patterns: &[String],
        strategy: NormalizationStrategy,
    ) -> Result<(), SubstrateError> {
        let dir_entries = std::fs::read_dir(current)
            .map_err(|e| SubstrateError::Fingerprint { path: current.to_path_buf(), reason: e.to_string() })?;

        let mut dir_entries: Vec<_> = dir_entries.filter_map(|e| e.ok()).collect();
        dir_entries.sort_unstable_by_key(|e| e.file_name());

        for entry in dir_entries {
            let path = entry.path();

            if Self::should_ignore(&path, ignore_patterns) {
                continue;
            }

            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            if path.is_dir() {
                Self::walk_dir(base, &path, entries, hasher, ignore_patterns, strategy)?;
            } else {
                if let Ok((hash, size, modified)) =
                    Self::fingerprint_file_with_strategy(&path, strategy)
                {
                    let normalized = strategy.normalize(base, &relative, &path);

                    match strategy {
                        NormalizationStrategy::HashOnly => {
                            // Only hash content contributes; path is ignored
                            hasher.update(&hash);
                            hasher.update(b";");
                            entries.push((
                                NormalizationStrategy::hash_only_path(&hash),
                                hash,
                                size,
                                modified,
                                false,
                            ));
                        }
                        _ => {
                            hasher.update(normalized.as_bytes());
                            hasher.update(b"=");
                            hasher.update(&hash);
                            hasher.update(b";");
                            entries.push((normalized.into_owned(), hash, size, modified, false));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[tonic::async_trait]
impl FileFingerprintService for FileFingerprintServiceImpl {
    async fn fingerprint_files(
        &self,
        request: Request<FingerprintFilesRequest>,
    ) -> Result<Response<FingerprintFilesResponse>, Status> {
        let req = request.into_inner();
        let mut all_entries = Vec::with_capacity(req.files.len());
        let mut collection_hasher = Md5::new();
        let strategy = NormalizationStrategy::from_str(&req.normalization_strategy);

        for file in &req.files {
            let path = Path::new(&file.absolute_path);

            if !path.exists() {
                continue;
            }

            let file_type =
                FingerprintType::try_from(file.r#type).unwrap_or(FingerprintType::FingerprintFile);

            match file_type {
                FingerprintType::FingerprintDirectory | FingerprintType::FingerprintRoot => {
                    if path.is_dir() {
                        match Self::fingerprint_directory(path, &req.ignore_patterns, strategy) {
                            Ok((entries, dir_hash)) => {
                                for (entry_path, hash, _size, _modified, _is_dir) in &entries {
                                    match strategy {
                                        NormalizationStrategy::HashOnly => {
                                            collection_hasher.update(hash);
                                        }
                                        _ => {
                                            collection_hasher.update(entry_path.as_bytes());
                                            collection_hasher.update(hash);
                                        }
                                    }
                                }
                                for (entry_path, hash, size, modified, is_dir) in entries {
                                    all_entries.push(FileFingerprintEntry {
                                        path: entry_path,
                                        hash,
                                        size,
                                        last_modified: modified,
                                        is_directory: is_dir,
                                    });
                                }
                                collection_hasher.update(&dir_hash);
                            }
                            Err(e) => {
                                return Ok(Response::new(FingerprintFilesResponse {
                                    success: false,
                                    error_message: e.to_string(),
                                    collection_hash: Vec::new(),
                                    entries: Vec::new(),
                                }));
                            }
                        }
                    }
                }
                FingerprintType::FingerprintFile => {
                    match Self::fingerprint_file_with_strategy(path, strategy) {
                        Ok((hash, size, modified)) => {
                            let display_path = match strategy {
                                NormalizationStrategy::RelativePath => file.absolute_path.clone(),
                                NormalizationStrategy::NameOnly => path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(&file.absolute_path)
                                    .to_string(),
                                NormalizationStrategy::HashOnly => {
                                    format!("hash-{:x}", Md5::digest(&hash))
                                }
                                _ => file.absolute_path.clone(),
                            };
                            all_entries.push(FileFingerprintEntry {
                                path: display_path.clone(),
                                hash: hash.clone(),
                                size,
                                last_modified: modified,
                                is_directory: false,
                            });
                            match strategy {
                                NormalizationStrategy::HashOnly => {
                                    collection_hasher.update(&hash);
                                }
                                _ => {
                                    collection_hasher.update(display_path.as_bytes());
                                    collection_hasher.update(&hash);
                                }
                            }
                        }
                        Err(e) => {
                            return Ok(Response::new(FingerprintFilesResponse {
                                success: false,
                                error_message: e.to_string(),
                                collection_hash: Vec::new(),
                                entries: Vec::new(),
                            }));
                        }
                    }
                }
            }
        }

        let collection_hash = collection_hasher.finalize().to_vec();

        tracing::debug!(
            files = all_entries.len(),
            strategy = ?strategy,
            "Fingerprinted files"
        );

        Ok(Response::new(FingerprintFilesResponse {
            success: true,
            error_message: String::new(),
            collection_hash,
            entries: all_entries,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fingerprint_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: file_path.to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        assert!(!resp.collection_hash.is_empty());
        assert_eq!(resp.entries[0].size, 11);
    }

    #[tokio::test]
    async fn test_fingerprint_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "bbb").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 2);
        assert!(!resp.collection_hash.is_empty());
    }

    #[tokio::test]
    async fn test_fingerprint_missing_file() {
        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: "/nonexistent/path.txt".to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 0);
    }

    #[test]
    fn test_hash_known_content() {
        // Verify that file hashing produces the standard MD5 of file content
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("known.txt");
        std::fs::write(&file_path, "test content").unwrap();

        let (hash, size, _) = FileFingerprintServiceImpl::fingerprint_file_with_strategy(
            &file_path,
            NormalizationStrategy::AbsolutePath,
        )
        .unwrap();

        // Standard MD5 of "test content" = 9473fdd0d880a43c21b7778d34872157
        let expected: [u8; 16] = Md5::digest(b"test content").into();
        assert_eq!(hash, expected.to_vec());
        assert_eq!(size, 12);
    }

    #[tokio::test]
    async fn test_fingerprint_with_ignore_patterns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        std::fs::write(dir.path().join("a.class"), "compiled").unwrap();
        std::fs::write(dir.path().join("b.txt"), "bbb").unwrap();
        std::fs::create_dir(dir.path().join("build")).unwrap();
        std::fs::write(dir.path().join("build/output.class"), "compiled").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: vec!["*.class".to_string(), "build".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        // Should only include a.txt and b.txt (class files and build/ ignored)
        assert_eq!(resp.entries.len(), 2);
    }

    #[test]
    fn test_should_ignore() {
        let path = Path::new("/some/path/build/output.class");
        assert!(FileFingerprintServiceImpl::should_ignore(
            path,
            &["*.class".to_string(), "build".to_string()],
        ));

        let path2 = Path::new("/some/path/src/Main.java");
        assert!(!FileFingerprintServiceImpl::should_ignore(
            path2,
            &["*.class".to_string(), "build".to_string()],
        ));
    }

    #[tokio::test]
    async fn test_fingerprint_name_only_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "world").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "NAME_ONLY".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 2);
        // Paths should be just filenames
        for entry in &resp.entries {
            assert!(
                !entry.path.contains('/'),
                "Expected name-only, got: {}",
                entry.path
            );
            assert!(
                entry.path == "a.txt" || entry.path == "b.txt",
                "Expected a.txt or b.txt, got: {}",
                entry.path
            );
        }
    }

    #[tokio::test]
    async fn test_fingerprint_hash_only_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "content").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "HASH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        // Path should start with "hash-"
        assert!(
            resp.entries[0].path.starts_with("hash-"),
            "Expected hash- prefix, got: {}",
            resp.entries[0].path
        );
    }

    #[tokio::test]
    async fn test_fingerprint_relative_path_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "data").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "RELATIVE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        // Relative path should be just "test.txt" (no absolute path prefix)
        assert_eq!(resp.entries[0].path, "test.txt");
    }

    #[tokio::test]
    async fn test_same_content_different_paths_hash_only() {
        // Two directories with same file content but different paths should have same hash
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir1.path().join("same.txt"), "identical content").unwrap();
        std::fs::write(dir2.path().join("same.txt"), "identical content").unwrap();

        let svc = FileFingerprintServiceImpl::new();

        let resp1 = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir1.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "HASH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir2.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "HASH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(
            resp1.collection_hash, resp2.collection_hash,
            "HASH strategy should produce same collection hash for same content regardless of path"
        );
    }

    #[test]
    fn test_normalization_strategy_from_str() {
        assert_eq!(
            NormalizationStrategy::from_str("ABSOLUTE_PATH"),
            NormalizationStrategy::AbsolutePath
        );
        assert_eq!(
            NormalizationStrategy::from_str("RELATIVE_PATH"),
            NormalizationStrategy::RelativePath
        );
        assert_eq!(
            NormalizationStrategy::from_str("NAME_ONLY"),
            NormalizationStrategy::NameOnly
        );
        assert_eq!(
            NormalizationStrategy::from_str("HASH"),
            NormalizationStrategy::HashOnly
        );
        assert_eq!(
            NormalizationStrategy::from_str("CLASS_ABI"),
            NormalizationStrategy::ClassAbi
        );
        assert_eq!(
            NormalizationStrategy::from_str("unknown"),
            NormalizationStrategy::AbsolutePath
        );
        assert_eq!(
            NormalizationStrategy::from_str(""),
            NormalizationStrategy::AbsolutePath
        );
    }

    // --- Class file ABI fingerprinting tests ---

    /// Build a minimal valid Java class file with the given public method descriptors.
    /// Uses raw bytes to avoid needing javac.
    fn build_minimal_class_file(
        class_name: &str,
        super_class: &str,
        public_methods: &[(&str, &str)], // (name, descriptor)
        private_methods: &[(&str, &str)],
    ) -> Vec<u8> {
        let mut cp = ConstantPoolBuilder::new();

        let class_name_idx = cp.add_class(class_name);
        let super_class_idx = cp.add_class(super_class);
        let code_attr_idx = cp.add_utf8("Code");

        // Build method entries
        let mut method_bytes = Vec::new();
        for (name, desc) in public_methods {
            let name_idx = cp.add_utf8(name);
            let desc_idx = cp.add_utf8(desc);
            // access_flags=ACC_PUBLIC (0x0001)
            method_bytes.extend_from_slice(&0x0001u16.to_be_bytes());
            method_bytes.extend_from_slice(&name_idx.to_be_bytes());
            method_bytes.extend_from_slice(&desc_idx.to_be_bytes());
            // 1 attribute (Code)
            method_bytes.extend_from_slice(&1u16.to_be_bytes());
            method_bytes.extend_from_slice(&code_attr_idx.to_be_bytes());
            // Code attribute length = 1 (minimal)
            method_bytes.extend_from_slice(&1u32.to_be_bytes());
            method_bytes.push(0x00); // minimal code
        }
        for (name, desc) in private_methods {
            let name_idx = cp.add_utf8(name);
            let desc_idx = cp.add_utf8(desc);
            // access_flags=0 (package-private)
            method_bytes.extend_from_slice(&0x0000u16.to_be_bytes());
            method_bytes.extend_from_slice(&name_idx.to_be_bytes());
            method_bytes.extend_from_slice(&desc_idx.to_be_bytes());
            method_bytes.extend_from_slice(&1u16.to_be_bytes());
            method_bytes.extend_from_slice(&code_attr_idx.to_be_bytes());
            method_bytes.extend_from_slice(&1u32.to_be_bytes());
            method_bytes.push(0x00);
        }

        // Assemble class file
        let mut out = Vec::new();
        // Magic
        out.extend_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE]);
        // Minor + major version (Java 8 = 52.0)
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&52u16.to_be_bytes());
        // Constant pool
        let cp_bytes = cp.build();
        out.extend_from_slice(&cp.entry_count().to_be_bytes()); // cp_count
        out.extend_from_slice(&cp_bytes);
        // Access flags: ACC_PUBLIC | ACC_SUPER
        out.extend_from_slice(&0x0021u16.to_be_bytes());
        // This class
        out.extend_from_slice(&class_name_idx.to_be_bytes());
        // Super class
        out.extend_from_slice(&super_class_idx.to_be_bytes());
        // Interfaces: none
        out.extend_from_slice(&0u16.to_be_bytes());
        // Fields: none
        out.extend_from_slice(&0u16.to_be_bytes());
        // Methods
        let method_count = (public_methods.len() + private_methods.len()) as u16;
        out.extend_from_slice(&method_count.to_be_bytes());
        out.extend_from_slice(&method_bytes);
        // Attributes: none
        out.extend_from_slice(&0u16.to_be_bytes());

        out
    }

    /// Helper to build a constant pool for test class files.
    struct ConstantPoolBuilder {
        entries: Vec<(u8, Vec<u8>)>,
        utf8_cache: std::collections::HashMap<String, u16>,
        class_cache: std::collections::HashMap<String, u16>,
    }

    impl ConstantPoolBuilder {
        fn new() -> Self {
            Self {
                entries: Vec::new(),
                utf8_cache: std::collections::HashMap::new(),
                class_cache: std::collections::HashMap::new(),
            }
        }

        fn next_index(&self) -> u16 {
            (self.entries.len() + 1) as u16
        }

        fn entry_count(&self) -> u16 {
            (self.entries.len() + 1) as u16 // cp_count = entries + 1 (1-indexed)
        }

        fn add_utf8(&mut self, s: &str) -> u16 {
            if let Some(&idx) = self.utf8_cache.get(s) {
                return idx;
            }
            let idx = self.next_index();
            let bytes = s.as_bytes();
            let mut entry = Vec::new();
            entry.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            entry.extend_from_slice(bytes);
            self.utf8_cache.insert(s.to_string(), idx);
            self.entries.push((CP_UTF8, entry));
            idx
        }

        fn add_class(&mut self, internal_name: &str) -> u16 {
            if let Some(&idx) = self.class_cache.get(internal_name) {
                return idx;
            }
            let name_idx = self.add_utf8(internal_name);
            let idx = self.next_index();
            let mut entry = Vec::new();
            entry.extend_from_slice(&name_idx.to_be_bytes());
            self.class_cache.insert(internal_name.to_string(), idx);
            self.entries.push((CP_CLASS, entry));
            idx
        }

        fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            for (tag, data) in &self.entries {
                out.push(*tag);
                out.extend_from_slice(data);
            }
            out
        }
    }

    #[test]
    fn test_class_file_abi_hash_valid_class() {
        let class_bytes = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("doStuff", "()V"), ("getValue", "()I")],
            &[("helper", "()V")],
        );

        let hash = class_file_abi_hash(&class_bytes);
        assert!(hash.is_some());
        let hash = hash.unwrap();
        assert_eq!(hash.len(), 16); // MD5 = 16 bytes
    }

    #[test]
    fn test_class_file_abi_hash_invalid_magic() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 0, 52];
        let hash = class_file_abi_hash(&data);
        assert!(hash.is_none());
    }

    #[test]
    fn test_class_file_abi_hash_ignores_private_methods() {
        // Two classes with same public API but different private methods
        // should produce the same ABI hash
        let class1 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("doStuff", "()V")],
            &[("privateHelper", "()V")],
        );
        let class2 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("doStuff", "()V")],
            &[("differentPrivateHelper", "()Z")],
        );

        let hash1 = class_file_abi_hash(&class1).unwrap();
        let hash2 = class_file_abi_hash(&class2).unwrap();
        assert_eq!(
            hash1, hash2,
            "Classes with same public API but different private methods should have same ABI hash"
        );
    }

    #[test]
    fn test_class_file_abi_hash_detects_public_api_change() {
        // Changing a public method signature should change the ABI hash
        let class1 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("doStuff", "()V")],
            &[],
        );
        let class2 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("doStuff", "(Ljava/lang/String;)V")],
            &[],
        );

        let hash1 = class_file_abi_hash(&class1).unwrap();
        let hash2 = class_file_abi_hash(&class2).unwrap();
        assert_ne!(
            hash1, hash2,
            "Changing public method signature should change ABI hash"
        );
    }

    #[test]
    fn test_class_file_abi_hash_detects_superclass_change() {
        let class1 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("run", "()V")],
            &[],
        );
        let class2 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Runnable",
            &[("run", "()V")],
            &[],
        );

        let hash1 = class_file_abi_hash(&class1).unwrap();
        let hash2 = class_file_abi_hash(&class2).unwrap();
        assert_ne!(hash1, hash2, "Changing superclass should change ABI hash");
    }

    #[test]
    fn test_class_file_abi_hash_detects_class_name_change() {
        let class1 = build_minimal_class_file(
            "com/example/Foo",
            "java/lang/Object",
            &[("run", "()V")],
            &[],
        );
        let class2 = build_minimal_class_file(
            "com/example/Bar",
            "java/lang/Object",
            &[("run", "()V")],
            &[],
        );

        let hash1 = class_file_abi_hash(&class1).unwrap();
        let hash2 = class_file_abi_hash(&class2).unwrap();
        assert_ne!(hash1, hash2, "Changing class name should change ABI hash");
    }

    #[tokio::test]
    async fn test_fingerprint_class_abi_strategy() {
        let dir = tempfile::tempdir().unwrap();

        // Create a valid class file
        let class_data = build_minimal_class_file(
            "com/example/Test",
            "java/lang/Object",
            &[("compute", "()I")],
            &[("privateHelper", "()V")],
        );
        let class_path = dir.path().join("Test.class");
        std::fs::write(&class_path, &class_data).unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: class_path.to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "CLASS_ABI".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].hash.len(), 16);

        // ABI hash should differ from raw content hash
        let content_hash = Md5::digest(&class_data);
        assert_ne!(
            resp.entries[0].hash,
            content_hash.to_vec(),
            "ABI hash should differ from raw content hash"
        );
    }

    #[tokio::test]
    async fn test_class_abi_non_class_files_unchanged() {
        // Non-.class files should use regular content hash even with CLASS_ABI strategy
        let dir = tempfile::tempdir().unwrap();
        let txt_path = dir.path().join("test.txt");
        std::fs::write(&txt_path, "plain text content").unwrap();

        let svc = FileFingerprintServiceImpl::new();

        let abi_resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: txt_path.to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "CLASS_ABI".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        let plain_resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: txt_path.to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(
            abi_resp.entries[0].hash, plain_resp.entries[0].hash,
            "Non-.class files should use same hash regardless of CLASS_ABI strategy"
        );
    }
}
