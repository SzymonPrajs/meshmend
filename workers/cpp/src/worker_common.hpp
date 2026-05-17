#pragma once

#include <cstdint>
#include <cctype>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>

namespace meshmend {

struct WorkerRequest {
  std::string operation_id;
  std::filesystem::path input_mesh;
  std::filesystem::path output_mesh;
  std::filesystem::path response_path;
  std::string operation;
};

inline std::string read_text(const std::filesystem::path &path) {
  std::ifstream input(path);
  if (!input) {
    throw std::runtime_error("failed to open " + path.string());
  }
  std::ostringstream buffer;
  buffer << input.rdbuf();
  return buffer.str();
}

inline std::optional<std::string> json_string(const std::string &json,
                                              const std::string &key) {
  const std::string needle = "\"" + key + "\"";
  const auto key_pos = json.find(needle);
  if (key_pos == std::string::npos) {
    return std::nullopt;
  }
  const auto colon = json.find(':', key_pos + needle.size());
  if (colon == std::string::npos) {
    return std::nullopt;
  }
  const auto first_quote = json.find('"', colon + 1);
  if (first_quote == std::string::npos) {
    return std::nullopt;
  }
  std::string value;
  bool escaped = false;
  for (auto i = first_quote + 1; i < json.size(); ++i) {
    const char c = json[i];
    if (escaped) {
      value.push_back(c);
      escaped = false;
      continue;
    }
    if (c == '\\') {
      escaped = true;
      continue;
    }
    if (c == '"') {
      return value;
    }
    value.push_back(c);
  }
  return std::nullopt;
}

inline std::optional<double> json_number(const std::string &json, const std::string &key) {
  const std::string needle = "\"" + key + "\"";
  const auto key_pos = json.find(needle);
  if (key_pos == std::string::npos) {
    return std::nullopt;
  }
  const auto colon = json.find(':', key_pos + needle.size());
  if (colon == std::string::npos) {
    return std::nullopt;
  }
  auto start = colon + 1;
  while (start < json.size() && std::isspace(static_cast<unsigned char>(json[start]))) {
    ++start;
  }
  if (json.compare(start, 4, "null") == 0) {
    return std::nullopt;
  }
  std::size_t consumed = 0;
  try {
    const double value = std::stod(json.substr(start), &consumed);
    if (consumed == 0) {
      return std::nullopt;
    }
    return value;
  } catch (const std::exception &) {
    return std::nullopt;
  }
}

inline WorkerRequest parse_request(const std::filesystem::path &request_path) {
  const auto json = read_text(request_path);
  WorkerRequest request;
  request.operation_id =
      json_string(json, "operation_id").value_or("00000000-0000-0000-0000-000000000000");
  request.operation = json_string(json, "operation").value_or("unknown");
  request.input_mesh = json_string(json, "input_mesh").value_or("");
  request.output_mesh = json_string(json, "output_mesh").value_or("");
  request.response_path = json_string(json, "response_path").value_or("response.json");
  return request;
}

inline std::uint32_t binary_stl_triangle_count(const std::filesystem::path &path) {
  std::ifstream input(path, std::ios::binary);
  if (!input) {
    throw std::runtime_error("failed to open STL " + path.string());
  }
  input.seekg(0, std::ios::end);
  const auto size = input.tellg();
  if (size < 84) {
    throw std::runtime_error("STL is too small");
  }
  input.seekg(80, std::ios::beg);
  std::uint32_t count = 0;
  input.read(reinterpret_cast<char *>(&count), sizeof(count));
  const auto expected = 84 + static_cast<std::uint64_t>(count) * 50;
  if (static_cast<std::uint64_t>(size) != expected) {
    throw std::runtime_error("STL byte size does not match declared triangle count");
  }
  return count;
}

inline void progress(const WorkerRequest &request, const std::string &event,
                     const std::string &phase, std::uint64_t current,
                     std::uint64_t total, const std::string &message) {
  std::cout << "{\"event\":\"" << event << "\","
            << "\"operation_id\":\"" << request.operation_id << "\","
            << "\"phase\":\"" << phase << "\","
            << "\"current\":" << current << ","
            << "\"total\":" << total << ","
            << "\"message\":\"" << message << "\","
            << "\"artifact_path\":null}" << std::endl;
}

inline void write_response(const WorkerRequest &request, bool success,
                           std::uint64_t input_triangles,
                           std::optional<std::uint64_t> output_triangles = std::nullopt,
                           const std::string &warning = "") {
  std::filesystem::create_directories(request.response_path.parent_path());
  std::ofstream output(request.response_path);
  output << "{\n";
  output << "  \"schema_version\": 1,\n";
  output << "  \"operation_id\": \"" << request.operation_id << "\",\n";
  output << "  \"success\": " << (success ? "true" : "false") << ",\n";
  if (success && !request.output_mesh.empty()) {
    output << "  \"output_mesh\": \"" << request.output_mesh.string() << "\",\n";
  } else {
    output << "  \"output_mesh\": null,\n";
  }
  output << "  \"changed_bounds\": null,\n";
  output << "  \"metrics\": {\n";
  output << "    \"input_triangles\": " << input_triangles << ",\n";
  if (output_triangles.has_value()) {
    output << "    \"output_triangles\": " << *output_triangles << ",\n";
  } else {
    output << "    \"output_triangles\": null,\n";
  }
  output << "    \"components\": null,\n";
  output << "    \"boundary_loops\": null,\n";
  output << "    \"non_manifold_edges\": null\n";
  output << "  },\n";
  output << "  \"warnings\": [";
  if (!warning.empty()) {
    output << "\"" << warning << "\"";
  }
  output << "],\n";
  output << "  \"validation\": {\n";
  output << "    \"closed\": null,\n";
  output << "    \"self_intersections\": null,\n";
  output << "    \"warnings\": []\n";
  output << "  },\n";
  output << "  \"error\": null\n";
  output << "}\n";
}

inline std::filesystem::path request_path_from_args(int argc, char **argv) {
  for (int i = 1; i + 1 < argc; ++i) {
    if (std::string(argv[i]) == "--request") {
      return argv[i + 1];
    }
  }
  throw std::runtime_error("usage: worker --request request.json");
}

} // namespace meshmend
