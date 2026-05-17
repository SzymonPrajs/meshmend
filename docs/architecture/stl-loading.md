# STL Loading

`crates/meshmend-stl` supports binary STL input only.

Validation policy:

- reject empty files
- reject files smaller than the 84-byte binary STL prefix
- reject ASCII STL with a clear error
- read the little-endian triangle count at byte offset 80
- require exact size `84 + triangle_count * 50`
- reject non-finite floats while parsing triangles

The parser memory maps the file with `memmap2`, divides records into chunks, and
uses `rayon` for parallel chunk parsing. The current chunk target is 100,000
triangles. Bounds are computed per chunk and reduced into global model bounds.

Local rose verification on this machine:

- triangles: 1,949,244
- source bytes: 97,462,284
- chunks: 20
