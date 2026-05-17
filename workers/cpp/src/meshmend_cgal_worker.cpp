#include "worker_common.hpp"

#include <CGAL/Simple_cartesian.h>
#include <CGAL/Surface_mesh.h>

int main(int argc, char **argv) {
  try {
    using Kernel = CGAL::Simple_cartesian<double>;
    using Point = Kernel::Point_3;
    CGAL::Surface_mesh<Point> smoke_mesh;
    (void)smoke_mesh;

    const auto request = meshmend::parse_request(meshmend::request_path_from_args(argc, argv));
    meshmend::progress(request, "started", "load", 0, 1, "CGAL worker started");
    const auto triangles = meshmend::binary_stl_triangle_count(request.input_mesh);
    meshmend::progress(request, "progress", "inspect", triangles, triangles,
                       "validated binary STL triangle count");
    meshmend::write_response(request, true, triangles);
    meshmend::progress(request, "done", "done", 1, 1, "CGAL worker finished");
    return 0;
  } catch (const std::exception &error) {
    std::cerr << error.what() << std::endl;
    return 1;
  }
}
