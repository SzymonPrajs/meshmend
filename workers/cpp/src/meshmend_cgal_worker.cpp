#include "worker_common.hpp"

#include <CGAL/boost/graph/iterator.h>
#include <CGAL/Polygon_mesh_processing/border.h>
#include <CGAL/Polygon_mesh_processing/triangulate_hole.h>
#include <CGAL/Simple_cartesian.h>
#include <CGAL/Surface_mesh.h>

#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <map>
#include <tuple>
#include <vector>

namespace {

using Kernel = CGAL::Simple_cartesian<double>;
using Point = Kernel::Point_3;
using Mesh = CGAL::Surface_mesh<Point>;
namespace PMP = CGAL::Polygon_mesh_processing;

struct VertexKey {
  std::int64_t x;
  std::int64_t y;
  std::int64_t z;

  bool operator<(const VertexKey &other) const {
    return std::tie(x, y, z) < std::tie(other.x, other.y, other.z);
  }
};

struct Vec3f {
  float x;
  float y;
  float z;
};

using TrianglePoints = std::array<Vec3f, 3>;

VertexKey key_for(const Vec3f &value) {
  constexpr double scale = 1.0e9;
  return {
      static_cast<std::int64_t>(std::llround(static_cast<double>(value.x) * scale)),
      static_cast<std::int64_t>(std::llround(static_cast<double>(value.y) * scale)),
      static_cast<std::int64_t>(std::llround(static_cast<double>(value.z) * scale)),
  };
}

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

Mesh::Vertex_index vertex_for(Mesh &mesh, std::map<VertexKey, Mesh::Vertex_index> &lookup,
                              const Vec3f &value) {
  const auto key = key_for(value);
  const auto found = lookup.find(key);
  if (found != lookup.end()) {
    return found->second;
  }
  const auto vertex = mesh.add_vertex(Point(value.x, value.y, value.z));
  lookup.emplace(key, vertex);
  return vertex;
}

std::vector<TrianglePoints> read_binary_stl_triangles(const std::filesystem::path &path) {
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

  std::vector<TrianglePoints> triangles;
  triangles.reserve(count);
  for (std::uint32_t triangle = 0; triangle < count; ++triangle) {
    const char *record = bytes.data() + 84 + static_cast<std::size_t>(triangle) * 50;
    triangles.push_back({{
        {read_f32(record + 12), read_f32(record + 16), read_f32(record + 20)},
        {read_f32(record + 24), read_f32(record + 28), read_f32(record + 32)},
        {read_f32(record + 36), read_f32(record + 40), read_f32(record + 44)},
    }});
  }
  return triangles;
}

Mesh mesh_from_triangles(const std::vector<TrianglePoints> &triangles) {
  Mesh mesh;
  std::map<VertexKey, Mesh::Vertex_index> vertices;
  for (const auto &points : triangles) {
    const auto a = vertex_for(mesh, vertices, points[0]);
    const auto b = vertex_for(mesh, vertices, points[1]);
    const auto c = vertex_for(mesh, vertices, points[2]);
    auto face = mesh.add_face(a, b, c);
    if (face == Mesh::null_face()) {
      face = mesh.add_face(a, c, b);
    }
    (void)face;
  }
  return mesh;
}

Mesh read_binary_stl_mesh(const std::filesystem::path &path) {
  return mesh_from_triangles(read_binary_stl_triangles(path));
}

std::array<float, 3> normal_for(const Point &a, const Point &b, const Point &c) {
  const double ux = b.x() - a.x();
  const double uy = b.y() - a.y();
  const double uz = b.z() - a.z();
  const double vx = c.x() - a.x();
  const double vy = c.y() - a.y();
  const double vz = c.z() - a.z();
  double nx = uy * vz - uz * vy;
  double ny = uz * vx - ux * vz;
  double nz = ux * vy - uy * vx;
  const double length = std::sqrt(nx * nx + ny * ny + nz * nz);
  if (length > 0.0) {
    nx /= length;
    ny /= length;
    nz /= length;
  }
  return {static_cast<float>(nx), static_cast<float>(ny), static_cast<float>(nz)};
}

void write_f32(std::ofstream &output, float value) {
  output.write(reinterpret_cast<const char *>(&value), sizeof(float));
}

void write_binary_stl(const Mesh &mesh, const std::filesystem::path &path) {
  std::filesystem::create_directories(path.parent_path());
  std::ofstream output(path, std::ios::binary);
  std::array<char, 80> header{};
  const std::string label = "MeshMend repaired binary STL";
  std::memcpy(header.data(), label.data(), label.size());
  output.write(header.data(), header.size());
  const auto face_count = static_cast<std::uint32_t>(mesh.number_of_faces());
  output.write(reinterpret_cast<const char *>(&face_count), sizeof(face_count));
  for (const auto face : mesh.faces()) {
    std::vector<Point> points;
    for (const auto vertex : CGAL::vertices_around_face(mesh.halfedge(face), mesh)) {
      points.push_back(mesh.point(vertex));
    }
    if (points.size() != 3) {
      continue;
    }
    const auto normal = normal_for(points[0], points[1], points[2]);
    for (const float value : normal) {
      write_f32(output, value);
    }
    for (const auto &point : points) {
      write_f32(output, static_cast<float>(point.x()));
      write_f32(output, static_cast<float>(point.y()));
      write_f32(output, static_cast<float>(point.z()));
    }
    const std::uint16_t attribute = 0;
    output.write(reinterpret_cast<const char *>(&attribute), sizeof(attribute));
  }
}

std::size_t fill_all_holes(Mesh &mesh) {
  std::vector<Mesh::Halfedge_index> border_cycles;
  PMP::extract_boundary_cycles(mesh, std::back_inserter(border_cycles));
  std::size_t patched_faces = 0;
  for (const auto halfedge : border_cycles) {
    std::vector<Mesh::Face_index> patch_facets;
    PMP::triangulate_hole(
        mesh, halfedge,
        CGAL::parameters::face_output_iterator(std::back_inserter(patch_facets)));
    patched_faces += patch_facets.size();
  }
  return patched_faces;
}

double signed_distance(const Vec3f &point, const std::array<double, 3> &normal, double offset) {
  return static_cast<double>(point.x) * normal[0] + static_cast<double>(point.y) * normal[1] +
         static_cast<double>(point.z) * normal[2] - offset;
}

Vec3f interpolate(const Vec3f &a, const Vec3f &b, double t) {
  return {
      static_cast<float>(static_cast<double>(a.x) +
                         (static_cast<double>(b.x) - static_cast<double>(a.x)) * t),
      static_cast<float>(static_cast<double>(a.y) +
                         (static_cast<double>(b.y) - static_cast<double>(a.y)) * t),
      static_cast<float>(static_cast<double>(a.z) +
                         (static_cast<double>(b.z) - static_cast<double>(a.z)) * t),
  };
}

std::vector<Vec3f> clip_polygon_to_plane(const TrianglePoints &triangle,
                                         const std::array<double, 3> &normal, double offset,
                                         bool keep_positive) {
  constexpr double epsilon = 1.0e-9;
  std::vector<Vec3f> input = {triangle[0], triangle[1], triangle[2]};
  std::vector<Vec3f> output;
  output.reserve(4);

  for (std::size_t i = 0; i < input.size(); ++i) {
    const Vec3f &current = input[i];
    const Vec3f &next = input[(i + 1) % input.size()];
    const double current_distance = signed_distance(current, normal, offset);
    const double next_distance = signed_distance(next, normal, offset);
    const bool current_kept =
        keep_positive ? current_distance >= -epsilon : current_distance <= epsilon;
    const bool next_kept = keep_positive ? next_distance >= -epsilon : next_distance <= epsilon;

    if (current_kept && next_kept) {
      output.push_back(next);
    } else if (current_kept && !next_kept) {
      const double denominator = current_distance - next_distance;
      const double t = denominator == 0.0 ? 0.0 : current_distance / denominator;
      output.push_back(interpolate(current, next, t));
    } else if (!current_kept && next_kept) {
      const double denominator = current_distance - next_distance;
      const double t = denominator == 0.0 ? 0.0 : current_distance / denominator;
      output.push_back(interpolate(current, next, t));
      output.push_back(next);
    }
  }

  return output;
}

std::vector<TrianglePoints> clip_triangles_to_plane(const std::vector<TrianglePoints> &triangles,
                                                    const std::array<double, 3> &normal,
                                                    double offset, bool keep_positive) {
  std::vector<TrianglePoints> clipped;
  clipped.reserve(triangles.size());
  for (const auto &triangle : triangles) {
    const auto polygon = clip_polygon_to_plane(triangle, normal, offset, keep_positive);
    if (polygon.size() < 3) {
      continue;
    }
    for (std::size_t index = 1; index + 1 < polygon.size(); ++index) {
      clipped.push_back({{polygon[0], polygon[index], polygon[index + 1]}});
    }
  }
  return clipped;
}

std::array<double, 3> normalized_plane_normal(const std::string &request_json) {
  const double nx = meshmend::json_number(request_json, "plane_nx").value_or(0.0);
  const double ny = meshmend::json_number(request_json, "plane_ny").value_or(0.0);
  const double nz = meshmend::json_number(request_json, "plane_nz").value_or(0.0);
  const double length = std::sqrt(nx * nx + ny * ny + nz * nz);
  if (length <= 0.0 || !std::isfinite(length)) {
    throw std::runtime_error("cut requires a non-zero plane normal");
  }
  return {nx / length, ny / length, nz / length};
}

} // namespace

int main(int argc, char **argv) {
  try {
    const auto request_path = meshmend::request_path_from_args(argc, argv);
    const auto request = meshmend::parse_request(request_path);
    meshmend::progress(request, "started", "load", 0, 1, "CGAL worker started");
    const auto triangles = meshmend::binary_stl_triangle_count(request.input_mesh);

    if (request.operation == "hole_fill") {
      meshmend::progress(request, "phase", "hole_fill", 0, triangles,
                         "loading mesh into CGAL Surface_mesh");
      auto mesh = read_binary_stl_mesh(request.input_mesh);
      const auto patched_faces = fill_all_holes(mesh);
      if (request.output_mesh.empty()) {
        throw std::runtime_error("hole_fill requires output_mesh");
      }
      write_binary_stl(mesh, request.output_mesh);
      meshmend::progress(request, "progress", "hole_fill", patched_faces, patched_faces,
                         "filled boundary cycles");
      meshmend::write_response(request, true, triangles, mesh.number_of_faces());
    } else if (request.operation == "cut") {
      if (request.output_mesh.empty()) {
        throw std::runtime_error("cut requires output_mesh");
      }
      const auto request_json = meshmend::read_text(request_path);
      const auto normal = normalized_plane_normal(request_json);
      const double offset = meshmend::json_number(request_json, "plane_offset").value_or(0.0);
      const std::string keep = meshmend::json_string(request_json, "keep").value_or("positive");
      const bool keep_positive = keep != "negative";

      meshmend::progress(request, "phase", "cut", 0, triangles,
                         "clipping triangle soup against cut plane");
      const auto clipped_triangles =
          clip_triangles_to_plane(read_binary_stl_triangles(request.input_mesh), normal, offset,
                                  keep_positive);
      auto mesh = mesh_from_triangles(clipped_triangles);
      meshmend::progress(request, "progress", "cut", clipped_triangles.size(), triangles,
                         "capping cut boundary cycles");
      const auto patched_faces = fill_all_holes(mesh);
      write_binary_stl(mesh, request.output_mesh);
      meshmend::progress(request, "progress", "cut", patched_faces, patched_faces,
                         "cut mesh capped and written");
      meshmend::write_response(request, true, triangles, mesh.number_of_faces());
    } else {
      meshmend::progress(request, "progress", "inspect", triangles, triangles,
                         "validated binary STL triangle count");
      meshmend::write_response(request, true, triangles);
    }

    meshmend::progress(request, "done", "done", 1, 1, "CGAL worker finished");
    return 0;
  } catch (const std::exception &error) {
    std::cerr << error.what() << std::endl;
    return 1;
  }
}
