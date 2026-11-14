//! Offline byte binding for the current single-file runtime archive contract.
//!
//! Verification establishes agreement with a validated manifest, not trusted provenance,
//! channel qualification, executable identity, or permission to stage, install or execute.

use crate::runtime_manifest::{require, ArtifactFormat, Component, ContractResult, RuntimeManifest};
use crate::runtime_status::ManagedIdentity;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

pub const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_RUNTIME_BYTES: u64 = 1 << 31;
const MAX_TAR_OVERHEAD: u64 = 64 * 1024;

/// Callers must choose memory budgets; neither field may exceed the hard ceiling.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactLimits {
    pub archive_bytes: u64,
    pub runtime_bytes: u64,
}

/// Manifest-bound bytes only. Construction does not establish provenance or authorization.
#[derive(Debug)]
pub struct VerifiedRuntimeArtifact {
    identity: ManagedIdentity,
    bytes: Vec<u8>,
}
impl VerifiedRuntimeArtifact {
    pub fn identity(&self) -> &ManagedIdentity {
        &self.identity
    }
    pub fn manifest_digest(&self) -> &str {
        self.identity.manifest_digest()
    }
    pub fn target(&self) -> &str {
        self.identity.target()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Verify exactly one selected runtime artifact, entirely in memory, without executing it.
/// Embedded build-info is not inspected: the identity returned is the manifest's claim.
/// ZIP support is limited to seekable classic single-member stored/deflated archives;
/// split archives, ZIP64, encryption and streaming data descriptors are rejected.
/// ZIP creator systems must be Unix (regular, non-shared, no special permissions)
/// or DOS (ordinary file attributes, optional regular-file mode). DOS permissions
/// are not interpreted as Unix access permissions. Tar permissions must be 0755.
pub fn verify_runtime_artifact(
    manifest: &RuntimeManifest,
    target: &str,
    archive: &[u8],
    limits: ArtifactLimits,
) -> ContractResult<VerifiedRuntimeArtifact> {
    require(
        limits.archive_bytes > 0
            && limits.archive_bytes <= MAX_ARCHIVE_BYTES
            && limits.runtime_bytes > 0
            && limits.runtime_bytes <= MAX_RUNTIME_BYTES,
        "invalid artifact limits",
    )?;
    let mut selected = manifest
        .artifacts()
        .iter()
        .filter(|a| a.component == Component::Runtime && a.target == target);
    let artifact = selected.next().ok_or("runtime target absent")?;
    require(selected.next().is_none(), "runtime target is not unique")?;
    require(
        artifact.size <= limits.archive_bytes && archive.len() as u64 <= limits.archive_bytes,
        "archive byte limit exceeded",
    )?;
    require(
        archive.len() as u64 == artifact.size,
        "archive size mismatch",
    )?;
    require(
        hash(archive) == artifact.sha256,
        "archive checksum mismatch",
    )?;
    let name = if target.contains("windows") {
        "refact.exe"
    } else {
        "refact"
    };
    require(
        artifact.extracted_files.len() == 1 && artifact.extracted_files[0].path == name,
        "manifest must bind exactly the root runtime file",
    )?;
    let file = &artifact.extracted_files[0];
    require(
        file.size <= limits.runtime_bytes,
        "runtime byte limit exceeded",
    )?;
    let bytes = match artifact.format {
        ArtifactFormat::TarGz => read_tar(archive, name, file.size)?,
        ArtifactFormat::Zip => read_zip(archive, name, file.size)?,
        _ => return Err("unsupported runtime archive format".into()),
    };
    require(bytes.len() as u64 == file.size, "runtime size mismatch")?;
    require(hash(&bytes) == file.sha256, "runtime checksum mismatch")?;
    Ok(VerifiedRuntimeArtifact {
        identity: ManagedIdentity::from_manifest(manifest, target)?,
        bytes,
    })
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn io<T>(value: std::io::Result<T>) -> ContractResult<T> {
    value.map_err(|e| e.to_string())
}
fn payload(reader: &mut impl Read, size: u64) -> ContractResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = io(reader.read(&mut buffer))?;
        if n == 0 {
            break;
        }
        require(
            (bytes.len() as u64).saturating_add(n as u64) <= size,
            "runtime size mismatch",
        )?;
        let required = bytes.len().checked_add(n).ok_or("runtime size overflow")?;
        if required > bytes.capacity() {
            let ceiling = usize::try_from(size).map_err(|_| "runtime size unsupported")?;
            let capacity = bytes
                .capacity()
                .saturating_mul(2)
                .max(buffer.len())
                .max(required)
                .min(ceiling);
            bytes
                .try_reserve_exact(capacity - bytes.len())
                .map_err(|_| "runtime allocation failed")?;
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    require(bytes.len() as u64 == size, "runtime size mismatch")?;
    Ok(bytes)
}
fn read_tar(bytes: &[u8], name: &str, size: u64) -> ContractResult<Vec<u8>> {
    let decoder = flate2::bufread::GzDecoder::new(Cursor::new(bytes));
    let budget = size + MAX_TAR_OVERHEAD;
    let mut archive = tar::Archive::new(decoder.take(budget + 1));
    let result;
    {
        let mut entries = io(archive.entries())?.raw(true);
        let mut entry = io(entries.next().ok_or("runtime member missing")?)?;
        require(
            entry.header().entry_type().is_file(),
            "tar member is not a regular file",
        )?;
        require(
            entry.path_bytes().as_ref() == name.as_bytes()
                && entry.header().link_name_bytes().is_none(),
            "tar member path/link rejected",
        )?;
        require(
            io(entry.header().mode())? == 0o755,
            "tar runtime permissions rejected",
        )?;
        require(entry.size() == size, "runtime size mismatch")?;
        result = payload(&mut entry, size)?;
        drop(entry);
        require(
            entries
                .next()
                .transpose()
                .map_err(|e| e.to_string())?
                .is_none(),
            "tar extra member rejected",
        )?;
    }
    let mut reader = archive.into_inner();
    let mut buffer = [0; 512];
    loop {
        let n = io(reader.read(&mut buffer))?;
        if n == 0 {
            break;
        }
        require(
            buffer[..n].iter().all(|b| *b == 0),
            "tar trailing member rejected",
        )?;
    }
    require(reader.limit() > 0, "tar uncompressed byte limit exceeded")?;
    require(
        reader.into_inner().into_inner().position() == bytes.len() as u64,
        "gzip trailing bytes rejected",
    )?;
    Ok(result)
}
fn zip_field<const N: usize>(bytes: &[u8], offset: usize) -> ContractResult<[u8; N]> {
    let end = offset.checked_add(N).ok_or("zip offset overflow")?;
    bytes
        .get(offset..end)
        .ok_or("zip field truncated")?
        .try_into()
        .map_err(|_| "zip field truncated".into())
}
fn u16_at(bytes: &[u8], offset: usize) -> ContractResult<u16> {
    Ok(u16::from_le_bytes(zip_field(bytes, offset)?))
}
fn u32_at(bytes: &[u8], offset: usize) -> ContractResult<u32> {
    Ok(u32::from_le_bytes(zip_field(bytes, offset)?))
}
fn zip_extent(start: usize, lengths: &[usize]) -> ContractResult<usize> {
    lengths.iter().try_fold(start, |end, len| {
        end.checked_add(*len)
            .ok_or_else(|| "zip offset overflow".into())
    })
}
fn read_zip(bytes: &[u8], name: &str, size: u64) -> ContractResult<Vec<u8>> {
    // Current producers write seekable classic ZIPs. Bind all record extents before
    // the library can ignore unreferenced local records or coalesce duplicate names.
    let end = (0..bytes.len().saturating_sub(21))
        .rev()
        .take(65536)
        .find(|&i| {
            bytes.get(i..i + 4) == Some(b"PK\x05\x06")
                && u16_at(bytes, i + 20).is_ok_and(|n| i + 22 + n as usize == bytes.len())
        })
        .ok_or("zip end record missing")?;
    require(
        u16_at(bytes, end + 4)? == 0
            && u16_at(bytes, end + 6)? == 0
            && u16_at(bytes, end + 8)? == 1
            && u16_at(bytes, end + 10)? == 1,
        "zip must contain exactly one member",
    )?;
    let central = u32_at(bytes, end + 16)? as usize;
    let central_size = u32_at(bytes, end + 12)? as usize;
    require(
        zip_extent(central, &[central_size])? == end,
        "zip central extent rejected",
    )?;
    let cd = bytes
        .get(central..end)
        .ok_or("zip central extent rejected")?;
    require(
        cd.get(..4) == Some(b"PK\x01\x02") && cd.len() >= 46,
        "zip central header rejected",
    )?;
    let central_name = u16_at(cd, 28)? as usize;
    require(
        zip_extent(
            46,
            &[
                central_name,
                u16_at(cd, 30)? as usize,
                u16_at(cd, 32)? as usize,
            ],
        )? == cd.len()
            && cd.get(46..zip_extent(46, &[central_name])?) == Some(name.as_bytes())
            && u16_at(cd, 34)? == 0
            && u32_at(cd, 42)? == 0
            && u16_at(cd, 6)? <= 20,
        "zip central record rejected",
    )?;
    let flags = u16_at(cd, 8)?;
    let method = u16_at(cd, 10)?;
    require(
        flags & !0x0806 == 0 && matches!(method, 0 | 8),
        "zip flags/method rejected",
    )?;
    require(
        bytes.get(..4) == Some(b"PK\x03\x04")
            && bytes.len() >= 30
            && u16_at(bytes, 4)? <= 20
            && u16_at(bytes, 6)? == flags
            && u16_at(bytes, 8)? == method
            && u32_at(bytes, 14)? == u32_at(cd, 16)?
            && u32_at(bytes, 18)? == u32_at(cd, 20)?
            && u32_at(bytes, 22)? == u32_at(cd, 24)?,
        "zip local/central mismatch",
    )?;
    let local_name = u16_at(bytes, 26)? as usize;
    let data_start = zip_extent(30, &[local_name, u16_at(bytes, 28)? as usize])?;
    let compressed = u32_at(cd, 20)?;
    require(
        compressed != u32::MAX
            && u32_at(cd, 24)? != u32::MAX
            && zip_extent(data_start, &[compressed as usize])? == central,
        "zip local extent rejected",
    )?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    require(archive.len() == 1, "zip must contain exactly one member")?;
    let mut file = archive.by_index(0).map_err(|e| e.to_string())?;
    require(
        file.header_start() == 0 && bytes.len() >= 30 && &bytes[..4] == b"PK\x03\x04",
        "zip local header rejected",
    )?;
    let name_len = u16_at(bytes, 26)? as usize;
    require(
        bytes.get(30..30 + name_len) == Some(name.as_bytes())
            && file.name_raw() == name.as_bytes()
            && !file.is_dir()
            && file.data_start() == data_start as u64
            && file.compressed_size() == compressed as u64,
        "zip member path/type/encryption rejected",
    )?;
    let attributes = u32_at(cd, 38)?;
    let mode = attributes >> 16;
    let permissions_ok = match cd[5] {
        3 => mode & 0o170000 == 0o100000 && mode & 0o7022 == 0 && attributes & 0xffff & !0x27 == 0,
        // Python writes st_mode in the high half even with a DOS creator system.
        0 => {
            attributes & 0xffff & !0x27 == 0
                && (mode == 0 || (mode & 0o170000 == 0o100000 && mode & 0o7000 == 0))
        }
        _ => false,
    };
    require(permissions_ok, "zip member permissions/type rejected")?;
    require(file.size() == size, "runtime size mismatch")?;
    payload(&mut file, size)
}
