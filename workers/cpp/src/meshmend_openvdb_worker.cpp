#include "worker_common.hpp"

#include <openvdb/openvdb.h>
#include <openvdb/tools/MeshToVolume.h>
#include <openvdb/tools/VolumeToMesh.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <limits>
#include <map>
#include <tuple>
#include <vector>

namespace {

struct VertexKey {
  std::int64_t x;
  std::int64_t y;
  std::int64_t z;

  bool operator<(const VertexKey &other) const {
    return std::tie(x, y, z) < std::tie(other.x, other.y, other.z);
  }
};

struct MeshVectors {
  std::vector<openvdb::Vec3s> points;
  std::vector<openvdb::Vec3I> triangles;
  openvdb::Vec3s bounds_min;
  openvdb::Vec3s bounds_max;
};

float read_f32(const char *data) {
  float value = 0.0f;
  std::memcpy(&value, data, sizeof(float));
  return value;
}

std::uint32_t read_u32(const char *data) {
  std::uint32_t value = 0;
  std::memcpy(&value, data, sizeof(std::uint32_t));
  return value;
}

void write_f32(std::ofstream &output, float value) {
  output.write(reinterpret_cast<const char *>(&value), sizeof(float));
}

VertexKey key_for(const openvdb::Vec3s &point) {
  constexpr double scale = 1.0e9;
  return {
      static_cast<std::int64_t>(std::llround(static_cast<double>(point.x()) * scale)),
      static_cast<std::int64_t>(std::llround(static_cast<double>(point.y()) * scale)),
      static_cast<std::int64_t>(std::llround(static_cast<double>(point.z()) * scale)),
  };
}

std::uint32_t vertex_for(MeshVectors &mesh, std::map<VertexKey, std::uint32_t> &lookup,
                         const openvdb::Vec3s &point) {
  const auto key = key_for(point);
  const auto found = lookup.find(key);
  if (found != lookup.end()) {
    return found->second;
  }
  const auto index = static_cast<std::uint32_t>(mesh.points.size());
  mesh.points.push_back(point);
  lookup.emplace(key, index);
  mesh.bounds_min = openvdb::Vec3s(std::min(mesh.bounds_min.x(), point.x()),
                                   std::min(mesh.bounds_min.y(), point.y()),
                                   std::min(mesh.bounds_min.z(), point.z()));
  mesh.bounds_max = openvdb::Vec3s(std::max(mesh.bounds_max.x(), point.x()),
                                   std::max(mesh.bounds_max.y(), point.y()),
                                   std::max(mesh.bounds_max.z(), point.z()));
  return index;
}

MeshVectors read_binary_stl_vectors(const std::filesystem::path &path) {
  std::ifstream input(path, std::ios::binary);
  if (!input) {
    throw std::runtime_error("failed to open STL " + path.string());
  }
  input.seekg(0, std::ios::end);
  const auto size = input.tellg();
  if (size < 84) {
    throw std::runtime_error("STL is too small");
  }
  input.seekg(0, std::ios::beg);
  std::vector<char> bytes(static_cast<std::size_t>(size));
  input.read(bytes.data(), static_cast<std::streamsize>(bytes.size()));
  const auto count = read_u32(bytes.data() + 80);
  const auto expected = 84 + static_cast<std::uint64_t>(count) * 50;
  if (expected != bytes.size()) {
    throw std::runtime_error("STL byte size does not match declared triangle count");
  }

  MeshVectors mesh;
  const float infinity = std::numeric_limits<float>::infinity();
  mesh.bounds_min = openvdb::Vec3s(infinity, infinity, infinity);
  mesh.bounds_max = openvdb::Vec3s(-infinity, -infinity, -infinity);
  mesh.triangles.reserve(count);
  std::map<VertexKey, std::uint32_t> vertices;

  for (std::uint32_t triangle = 0; triangle < count; ++triangle) {
    const char *record = bytes.data() + 84 + static_cast<std::size_t>(triangle) * 50;
    const std::array<openvdb::Vec3s, 3> points = {{
        {read_f32(record + 12), read_f32(record + 16), read_f32(record + 20)},
        {read_f32(record + 24), read_f32(record + 28), read_f32(record + 32)},
        {read_f32(record + 36), read_f32(record + 40), read_f32(record + 44)},
    }};
    const auto a = vertex_for(mesh, vertices, points[0]);
    const auto b = vertex_for(mesh, vertices, points[1]);
    const auto c = vertex_for(mesh, vertices, points[2]);
    if (a != b && b != c && c != a) {
      mesh.triangles.emplace_back(static_cast<int>(a), static_cast<int>(b), static_cast<int>(c));
    }
  }

  return mesh;
}

std::array<float, 3> normal_for(const openvdb::Vec3s &a, const openvdb::Vec3s &b,
                                const openvdb::Vec3s &c) {
  const openvdb::Vec3s normal = (b - a).cross(c - a);
  const float length = normal.length();
  if (length <= 0.0f) {
    return {0.0f, 0.0f, 0.0f};
  }
  return {normal.x() / length, normal.y() / length, normal.z() / length};
}

void write_triangle(std::ofstream &output, const openvdb::Vec3s &a, const openvdb::Vec3s &b,
                    const openvdb::Vec3s &c) {
  const auto normal = normal_for(a, b, c);
  for (const float value : normal) {
    write_f32(output, value);
  }
  for (const auto &point : {a, b, c}) {
    write_f32(output, point.x());
    write_f32(output, point.y());
    write_f32(output, point.z());
  }
  const std::uint16_t attribute = 0;
  output.write(reinterpret_cast<const char *>(&attribute), sizeof(attribute));
}

std::uint64_t write_binary_stl(const std::vector<openvdb::Vec3s> &points,
                               const std::vector<openvdb::Vec3I> &triangles,
                               const std::vector<openvdb::Vec4I> &quads,
                               const std::filesystem::path &path) {
  const auto parent = path.parent_path();
  if (!parent.empty()) {
    std::filesystem::create_directories(parent);
  }
  const std::uint64_t triangle_count =
      static_cast<std::uint64_t>(triangles.size()) + static_cast<std::uint64_t>(quads.size()) * 2;
  if (triangle_count > std::numeric_limits<std::uint32_t>::max()) {
    throw std::runtime_error("VDB mesh is too large for binary STL");
  }

  std::ofstream output(path, std::ios::binary);
  std::array<char, 80> header{};
  const std::string label = "MeshMend OpenVDB wrapped binary STL";
  std::memcpy(header.data(), label.data(), label.size());
  output.write(header.data(), header.size());
  const auto triangle_count_u32 = static_cast<std::uint32_t>(triangle_count);
  output.write(reinterpret_cast<const char *>(&triangle_count_u32), sizeof(triangle_count_u32));

  for (const auto &triangle : triangles) {
    write_triangle(output, points[triangle.x()], points[triangle.y()], points[triangle.z()]);
  }
  for (const auto &quad : quads) {
    write_triangle(output, points[quad.x()], points[quad.y()], points[quad.z()]);
    write_triangle(output, points[quad.x()], points[quad.z()], points[quad.w()]);
  }

  return triangle_count;
}

double mesh_diagonal(const MeshVectors &mesh) {
  const double dx = static_cast<double>(mesh.bounds_max.x() - mesh.bounds_min.x());
  const double dy = static_cast<double>(mesh.bounds_max.y() - mesh.bounds_min.y());
  const double dz = static_cast<double>(mesh.bounds_max.z() - mesh.bounds_min.z());
  return std::sqrt(dx * dx + dy * dy + dz * dz);
}

double choose_voxel_size_from_request_json(const std::string &request_json,
                                           const MeshVectors &mesh) {
  const double diagonal = std::max(mesh_diagonal(mesh), 1.0e-6);
  const auto target_edge_length = meshmend::json_number(request_json, "target_edge_length");
  if (target_edge_length.has_value() && *target_edge_length > 0.0) {
    return *target_edge_length;
  }
  return diagonal / 48.0;
}

std::uint64_t run_local_sdf_wrap(const meshmend::WorkerRequest &request,
                                 const std::filesystem::path &request_path) {
  if (request.output_mesh.empty()) {
    throw std::runtime_error("local_sdf_wrap requires output_mesh");
  }

  meshmend::progress(request, "phase", "local_sdf_wrap", 0, 5,
                     "loading STL as OpenVDB mesh vectors");
  auto mesh = read_binary_stl_vectors(request.input_mesh);
  if (mesh.points.empty() || mesh.triangles.empty()) {
    throw std::runtime_error("input mesh has no usable triangles");
  }

  const auto request_json = meshmend::read_text(request_path);
  const double voxel_size = choose_voxel_size_from_request_json(request_json, mesh);
  if (!std::isfinite(voxel_size) || voxel_size <= 0.0) {
    throw std::runtime_error("invalid VDB voxel size");
  }
  meshmend::progress(request, "progress", "local_sdf_wrap", 1, 5,
                     "converting mesh to OpenVDB level set");
  auto transform = openvdb::math::Transform::createLinearTransform(voxel_size);
  auto grid =
      openvdb::tools::meshToLevelSet<openvdb::FloatGrid>(*transform, mesh.points, mesh.triangles,
                                                         3.0f);

  meshmend::progress(request, "progress", "local_sdf_wrap", 3, 5,
                     "extracting wrapped surface from level set");
  std::vector<openvdb::Vec3s> output_points;
  std::vector<openvdb::Vec3I> output_triangles;
  std::vector<openvdb::Vec4I> output_quads;
  openvdb::tools::volumeToMesh(*grid, output_points, output_triangles, output_quads, 0.0, 0.0,
                               true);
  if (output_points.empty() || (output_triangles.empty() && output_quads.empty())) {
    throw std::runtime_error("OpenVDB produced an empty surface");
  }

  meshmend::progress(request, "progress", "local_sdf_wrap", 4, 5,
                     "writing OpenVDB wrapped STL");
  const auto output_triangles_written =
      write_binary_stl(output_points, output_triangles, output_quads, request.output_mesh);
  meshmend::progress(request, "progress", "local_sdf_wrap", 5, 5,
                     "OpenVDB local wrap complete");
  return output_triangles_written;
}

} // namespace

int main(int argc, char **argv) {
  try {
    openvdb::initialize();
    const auto request_path = meshmend::request_path_from_args(argc, argv);
    const auto request = meshmend::parse_request(request_path);
    meshmend::progress(request, "started", "load", 0, 1, "OpenVDB worker started");
    const auto triangles = meshmend::binary_stl_triangle_count(request.input_mesh);

    if (request.operation == "local_sdf_wrap") {
      const auto output_triangles = run_local_sdf_wrap(request, request_path);
      meshmend::write_response(request, true, triangles, output_triangles);
    } else {
      meshmend::progress(request, "progress", "inspect", triangles, triangles,
                         "validated binary STL before VDB operation");
      meshmend::write_response(request, true, triangles);
    }

    meshmend::progress(request, "done", "done", 1, 1, "OpenVDB worker finished");
    openvdb::uninitialize();
    return 0;
  } catch (const std::exception &error) {
    std::cerr << error.what() << std::endl;
    return 1;
  }
}
