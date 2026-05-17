#include "worker_common.hpp"

#include <openvdb/openvdb.h>

int main(int argc, char **argv) {
  try {
    openvdb::initialize();
    const auto request = meshmend::parse_request(meshmend::request_path_from_args(argc, argv));
    meshmend::progress(request, "started", "load", 0, 1, "OpenVDB worker started");
    const auto triangles = meshmend::binary_stl_triangle_count(request.input_mesh);
    meshmend::progress(request, "progress", "inspect", triangles, triangles,
                       "validated binary STL before VDB operation");
    meshmend::write_response(request, true, triangles);
    meshmend::progress(request, "done", "done", 1, 1, "OpenVDB worker finished");
    openvdb::uninitialize();
    return 0;
  } catch (const std::exception &error) {
    std::cerr << error.what() << std::endl;
    return 1;
  }
}
