use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use glam::Vec3;
use memmap2::MmapOptions;
use meshmend_core::{MeshBounds, MeshStats, Triangle, TriangleId};
use rayon::prelude::*;

const HEADER_BYTES: usize = 80;
const COUNT_BYTES: usize = 4;
const PREFIX_BYTES: usize = HEADER_BYTES + COUNT_BYTES;
const TRIANGLE_RECORD_BYTES: usize = 50;
pub const DEFAULT_CHUNK_TRIANGLES: usize = 100_000;

#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub chunk_triangles: usize,
    pub parallel: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            chunk_triangles: DEFAULT_CHUNK_TRIANGLES,
            parallel: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TriangleChunk {
    pub chunk_index: u32,
    pub start_triangle: u64,
    pub bounds: MeshBounds,
    pub triangles: Vec<Triangle>,
}

#[derive(Debug, Clone)]
pub struct ParsedStl {
    pub source_path: PathBuf,
    pub file_name: String,
    pub source_bytes: u64,
    pub stats: MeshStats,
    pub timings: StlTimings,
    pub chunks: Vec<TriangleChunk>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StlTimings {
    pub map_file: Duration,
    pub validate: Duration,
    pub parse: Duration,
}

pub fn load_binary_stl(path: &Path) -> Result<ParsedStl, StlError> {
    load_binary_stl_with_options(path, &LoadOptions::default())
}

pub fn write_binary_stl(path: &Path, triangles: &[Triangle]) -> Result<(), StlError> {
    if triangles.len() > u32::MAX as usize {
        return Err(StlError::TooManyTriangles {
            triangle_count: triangles.len(),
        });
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = File::create(path)?;
    let mut header = [0_u8; HEADER_BYTES];
    let label = b"MeshMend binary STL";
    header[..label.len()].copy_from_slice(label);
    output.write_all(&header)?;
    output.write_all(&(triangles.len() as u32).to_le_bytes())?;
    for triangle in triangles {
        write_vec3(&mut output, computed_normal(*triangle))?;
        for vertex in triangle.vertices {
            write_vec3(&mut output, vertex)?;
        }
        output.write_all(&0_u16.to_le_bytes())?;
    }
    Ok(())
}

pub fn load_binary_stl_with_options(
    path: &Path,
    options: &LoadOptions,
) -> Result<ParsedStl, StlError> {
    let started_map = std::time::Instant::now();
    let file = File::open(path)?;
    let source_bytes = file.metadata()?.len();
    if source_bytes == 0 {
        return Err(StlError::EmptyFile);
    }
    let map = unsafe { MmapOptions::new().map(&file)? };
    let map_file = started_map.elapsed();

    let started_validate = std::time::Instant::now();
    let triangle_count = validate_binary_stl(&map)?;
    let validate = started_validate.elapsed();

    let started_parse = std::time::Instant::now();
    let chunks = parse_chunks(&map, triangle_count, options)?;
    let parse = started_parse.elapsed();

    let bounds = chunks.iter().fold(MeshBounds::EMPTY, |bounds, chunk| {
        bounds.union(chunk.bounds)
    });

    let stats = MeshStats {
        triangle_count: triangle_count as u64,
        vertex_position_count: triangle_count as u64 * 3,
        bounds,
        source_bytes,
    };

    Ok(ParsedStl {
        source_path: path.to_path_buf(),
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        source_bytes,
        stats,
        timings: StlTimings {
            map_file,
            validate,
            parse,
        },
        chunks,
    })
}

pub fn validate_binary_stl(bytes: &[u8]) -> Result<u32, StlError> {
    if bytes.is_empty() {
        return Err(StlError::EmptyFile);
    }
    if looks_like_ascii_stl(bytes) {
        return Err(StlError::AsciiUnsupported);
    }
    if bytes.len() < PREFIX_BYTES {
        return Err(StlError::TooSmall {
            actual_bytes: bytes.len() as u64,
        });
    }

    let declared = u32::from_le_bytes(bytes[HEADER_BYTES..PREFIX_BYTES].try_into().unwrap());
    let expected = expected_size(declared)?;
    let actual = bytes.len() as u64;

    if expected != actual {
        return Err(StlError::InvalidTriangleCount {
            declared,
            expected_bytes: expected,
            actual_bytes: actual,
        });
    }

    Ok(declared)
}

fn expected_size(triangle_count: u32) -> Result<u64, StlError> {
    let records = u64::from(triangle_count)
        .checked_mul(TRIANGLE_RECORD_BYTES as u64)
        .ok_or(StlError::CountOverflow { triangle_count })?;
    (PREFIX_BYTES as u64)
        .checked_add(records)
        .ok_or(StlError::CountOverflow { triangle_count })
}

fn parse_chunks(
    bytes: &[u8],
    triangle_count: u32,
    options: &LoadOptions,
) -> Result<Vec<TriangleChunk>, StlError> {
    let chunk_triangles = options
        .chunk_triangles
        .clamp(1, TriangleId::LOCAL_MASK as usize + 1);
    let chunk_ranges = chunk_ranges(triangle_count as usize, chunk_triangles)?;

    if options.parallel && chunk_ranges.len() > 1 {
        chunk_ranges
            .into_par_iter()
            .map(|(chunk_index, start, end)| parse_chunk(bytes, chunk_index, start, end))
            .collect()
    } else {
        chunk_ranges
            .into_iter()
            .map(|(chunk_index, start, end)| parse_chunk(bytes, chunk_index, start, end))
            .collect()
    }
}

fn chunk_ranges(
    triangle_count: usize,
    chunk_triangles: usize,
) -> Result<Vec<(u32, usize, usize)>, StlError> {
    let chunk_count = triangle_count.div_ceil(chunk_triangles);
    if chunk_count > (u32::MAX as usize) {
        return Err(StlError::TooManyChunks {
            chunk_count,
            chunk_triangles,
        });
    }

    let mut ranges = Vec::with_capacity(chunk_count);
    for chunk_index in 0..chunk_count {
        let start = chunk_index * chunk_triangles;
        let end = (start + chunk_triangles).min(triangle_count);
        ranges.push((chunk_index as u32, start, end));
    }
    Ok(ranges)
}

fn parse_chunk(
    bytes: &[u8],
    chunk_index: u32,
    start: usize,
    end: usize,
) -> Result<TriangleChunk, StlError> {
    let mut bounds = MeshBounds::EMPTY;
    let mut triangles = Vec::with_capacity(end - start);

    for triangle_index in start..end {
        let offset = PREFIX_BYTES + triangle_index * TRIANGLE_RECORD_BYTES;
        let record = &bytes[offset..offset + TRIANGLE_RECORD_BYTES];
        let triangle = parse_triangle(record, triangle_index as u64)?;
        for vertex in triangle.vertices {
            bounds.include_point(vertex);
        }
        triangles.push(triangle);
    }

    Ok(TriangleChunk {
        chunk_index,
        start_triangle: start as u64,
        bounds,
        triangles,
    })
}

fn parse_triangle(record: &[u8], triangle_index: u64) -> Result<Triangle, StlError> {
    let normal = read_vec3(record, 0, triangle_index)?;
    let vertices = [
        read_vec3(record, 12, triangle_index)?,
        read_vec3(record, 24, triangle_index)?,
        read_vec3(record, 36, triangle_index)?,
    ];

    Ok(Triangle { normal, vertices })
}

fn read_vec3(record: &[u8], offset: usize, triangle_index: u64) -> Result<Vec3, StlError> {
    let x = read_f32(record, offset);
    let y = read_f32(record, offset + 4);
    let z = read_f32(record, offset + 8);
    let value = Vec3::new(x, y, z);
    if !value.is_finite() {
        return Err(StlError::NonFiniteFloat { triangle_index });
    }
    Ok(value)
}

fn read_f32(record: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap())
}

fn write_vec3(output: &mut File, value: Vec3) -> Result<(), StlError> {
    output.write_all(&value.x.to_le_bytes())?;
    output.write_all(&value.y.to_le_bytes())?;
    output.write_all(&value.z.to_le_bytes())?;
    Ok(())
}

fn computed_normal(triangle: Triangle) -> Vec3 {
    let normal = (triangle.vertices[1] - triangle.vertices[0])
        .cross(triangle.vertices[2] - triangle.vertices[0])
        .normalize_or_zero();
    if normal.length_squared() > f32::EPSILON {
        normal
    } else {
        triangle.normal.normalize_or_zero()
    }
}

fn looks_like_ascii_stl(bytes: &[u8]) -> bool {
    let prefix = &bytes[..bytes.len().min(256)];
    let trimmed = prefix
        .iter()
        .copied()
        .skip_while(|byte| byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    trimmed
        .get(..5)
        .is_some_and(|start| start.eq_ignore_ascii_case(b"solid"))
        && prefix.iter().all(|byte| {
            byte.is_ascii_whitespace() || (byte.is_ascii_graphic() && !byte.is_ascii_control())
        })
}

#[derive(Debug, thiserror::Error)]
pub enum StlError {
    #[error("file is empty")]
    EmptyFile,
    #[error("file is too small to be a binary STL: {actual_bytes} bytes")]
    TooSmall { actual_bytes: u64 },
    #[error("ASCII STL is not supported yet; use binary STL")]
    AsciiUnsupported,
    #[error(
        "declared STL triangle count {declared} expects {expected_bytes} bytes, but file has {actual_bytes} bytes"
    )]
    InvalidTriangleCount {
        declared: u32,
        expected_bytes: u64,
        actual_bytes: u64,
    },
    #[error("triangle count {triangle_count} overflows STL byte size calculation")]
    CountOverflow { triangle_count: u32 },
    #[error("parsed triangle {triangle_index} contains a non-finite float")]
    NonFiniteFloat { triangle_index: u64 },
    #[error(
        "chunk configuration would create {chunk_count} chunks of {chunk_triangles} triangles"
    )]
    TooManyChunks {
        chunk_count: usize,
        chunk_triangles: usize,
    },
    #[error("too many triangles to write binary STL: {triangle_count}")]
    TooManyTriangles { triangle_count: usize },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cube_fixture() {
        let parsed = load_binary_stl(Path::new("../../fixtures/stl/cube_binary.stl"))
            .expect("cube fixture should parse");

        assert_eq!(parsed.stats.triangle_count, 12);
        assert_eq!(parsed.stats.vertex_position_count, 36);
        assert_eq!(parsed.chunks.len(), 1);
        assert_eq!(parsed.stats.bounds.min, Vec3::new(-1.0, -1.0, -1.0));
        assert_eq!(parsed.stats.bounds.max, Vec3::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn rejects_ascii_stl() {
        let bytes = std::fs::read("../../fixtures/stl/malformed_header.stl").unwrap();
        let err = validate_binary_stl(&bytes).unwrap_err();
        assert!(matches!(err, StlError::AsciiUnsupported));
    }

    #[test]
    fn rejects_invalid_count() {
        let bytes = std::fs::read("../../fixtures/stl/invalid_count.stl").unwrap();
        let err = validate_binary_stl(&bytes).unwrap_err();
        assert!(matches!(err, StlError::InvalidTriangleCount { .. }));
    }

    #[test]
    fn splits_chunks() {
        let parsed = load_binary_stl_with_options(
            Path::new("../../fixtures/stl/cube_binary.stl"),
            &LoadOptions {
                chunk_triangles: 5,
                parallel: false,
            },
        )
        .expect("cube fixture should parse");

        assert_eq!(parsed.chunks.len(), 3);
        assert_eq!(parsed.chunks[0].start_triangle, 0);
        assert_eq!(parsed.chunks[1].start_triangle, 5);
        assert_eq!(parsed.chunks[2].triangles.len(), 2);
    }

    #[test]
    fn writes_binary_stl_that_reloads() {
        let parsed = load_binary_stl(Path::new("../../fixtures/stl/cube_binary.stl"))
            .expect("cube fixture should parse");
        let triangles = parsed
            .chunks
            .iter()
            .flat_map(|chunk| chunk.triangles.iter().copied())
            .collect::<Vec<_>>();
        let output =
            std::env::temp_dir().join(format!("meshmend-stl-write-{}.stl", std::process::id()));

        write_binary_stl(&output, &triangles).expect("write should succeed");
        let reloaded = load_binary_stl(&output).expect("written STL should reload");

        assert_eq!(reloaded.stats.triangle_count, parsed.stats.triangle_count);
        let _ = std::fs::remove_file(output);
    }
}
