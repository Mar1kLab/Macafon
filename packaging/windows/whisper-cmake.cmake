# This project-specific hook runs after whisper.cpp's project() initializes the
# compiler flags. cmake-rs otherwise replaces MSVC Release flags with cc-rs's
# deliberately unoptimized flags. Apply only to Whisper, not other native crates.
if(MSVC)
    set(CMAKE_C_FLAGS_RELEASE "/O2 /DNDEBUG" CACHE STRING "Whisper release C flags" FORCE)
    set(CMAKE_CXX_FLAGS_RELEASE "/O2 /DNDEBUG" CACHE STRING "Whisper release C++ flags" FORCE)
endif()

if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(AMD64|amd64|x86_64)$")
    # Ship one portable AVX2 baseline, never the build host's native ISA. Static
    # whisper-rs does not ship ggml's dynamically loaded CPU variant backends.
    set(GGML_NATIVE OFF CACHE BOOL "Portable Whisper CPU build" FORCE)
    set(GGML_AVX ON CACHE BOOL "AVX baseline" FORCE)
    set(GGML_AVX2 ON CACHE BOOL "AVX2 baseline" FORCE)
    set(GGML_FMA ON CACHE BOOL "FMA baseline" FORCE)
    set(GGML_F16C ON CACHE BOOL "F16C baseline" FORCE)
    # These features are not part of the installer CPU baseline.
    foreach(feature AVX_VNNI AVX512 AVX512_VBMI AVX512_VNNI AVX512_BF16 AMX_TILE AMX_INT8 AMX_BF16 BMI2)
        set(GGML_${feature} OFF CACHE BOOL "Outside the portable AVX2 baseline" FORCE)
    endforeach()
endif()
