fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // sherpa-onnx copies its shared runtime next to the executable. Let the daemon
        // launch the worker without modifying the user's loader environment.
        println!("cargo:rustc-link-arg-bin=macafon-stt=-Wl,-rpath,$ORIGIN");
    }
}
