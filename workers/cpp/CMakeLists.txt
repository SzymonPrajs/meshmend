cmake_minimum_required(VERSION 3.24)
project(meshmend_workers LANGUAGES CXX)

if(POLICY CMP0167)
  cmake_policy(SET CMP0167 NEW)
  set(CMAKE_POLICY_DEFAULT_CMP0167 NEW)
endif()

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

execute_process(
  COMMAND brew --prefix openvdb
  OUTPUT_VARIABLE HOMEBREW_OPENVDB_PREFIX
  OUTPUT_STRIP_TRAILING_WHITESPACE
  ERROR_QUIET
)
if(HOMEBREW_OPENVDB_PREFIX)
  list(APPEND CMAKE_MODULE_PATH "${HOMEBREW_OPENVDB_PREFIX}/lib/cmake/OpenVDB")
endif()

find_package(CGAL REQUIRED)
find_package(OpenVDB REQUIRED MODULE)

add_executable(meshmend-cgal-worker
  src/meshmend_cgal_worker.cpp
)
target_link_libraries(meshmend-cgal-worker PRIVATE CGAL::CGAL)

add_executable(meshmend-openvdb-worker
  src/meshmend_openvdb_worker.cpp
)
target_link_libraries(meshmend-openvdb-worker PRIVATE OpenVDB::openvdb)
