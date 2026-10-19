//! Reviewed, immutable Hugging Face downloads. Hashes are SHA-256 of file contents,
//! not Git blob IDs (metadata verified through the Hugging Face API).
use crate::protocol::Engine;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub repo: &'static str,
    pub revision: &'static str,
    pub path: &'static str,
    pub file_name: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
}

impl ModelFile {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.path
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub engine: Engine,
    pub name: &'static str,
    pub quality: u8,
    pub speed: u8,
    pub recommended: bool,
    pub files: &'static [ModelFile],
}

impl ModelSpec {
    pub fn size_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size_bytes).sum()
    }
}

pub fn model_dir(models_dir: impl AsRef<Path>, spec: &ModelSpec) -> PathBuf {
    models_dir
        .as_ref()
        .join(match spec.engine {
            Engine::Whisper => "whisper",
            Engine::Parakeet => "parakeet",
        })
        .join(spec.id)
}

const WHISPER_REPO: &str = "ggerganov/whisper.cpp";
const WHISPER_REV: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const PARAKEET_REPO: &str = "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
const PARAKEET_REV: &str = "2bda32ec70b097a55adaa07d9a7173915b43cc78";
const WHISPER_VAD: ModelFile = ModelFile {
    repo: "ggml-org/whisper-vad",
    revision: "9ffd54a1e1ee413ddf265af9913beaf518d1639b",
    path: "ggml-silero-v5.1.2.bin",
    file_name: "ggml-silero-v5.1.2.bin",
    size_bytes: 885098,
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
};

pub fn models() -> &'static [ModelSpec] {
    &[
        ModelSpec {
            id: "whisper-large-v3-turbo-q5_0",
            engine: Engine::Whisper,
            name: "Turbo",
            quality: 5,
            speed: 2,
            recommended: true,
            files: &[
                ModelFile {
                    repo: WHISPER_REPO,
                    revision: WHISPER_REV,
                    path: "ggml-large-v3-turbo-q5_0.bin",
                    file_name: "ggml-large-v3-turbo-q5_0.bin",
                    size_bytes: 574041195,
                    sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
                },
                WHISPER_VAD,
            ],
        },
        ModelSpec {
            id: "whisper-small-q5_1",
            engine: Engine::Whisper,
            name: "Small",
            quality: 2,
            speed: 4,
            recommended: false,
            files: &[
                ModelFile {
                    repo: WHISPER_REPO,
                    revision: WHISPER_REV,
                    path: "ggml-small-q5_1.bin",
                    file_name: "ggml-small-q5_1.bin",
                    size_bytes: 190085487,
                    sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
                },
                WHISPER_VAD,
            ],
        },
        ModelSpec {
            id: "parakeet-tdt-0.6b-v3-int8",
            engine: Engine::Parakeet,
            name: "Parakeet v3",
            quality: 4,
            speed: 5,
            recommended: false,
            files: &[
                ModelFile {
                    repo: PARAKEET_REPO,
                    revision: PARAKEET_REV,
                    path: "encoder.int8.onnx",
                    file_name: "encoder.int8.onnx",
                    size_bytes: 652184281,
                    sha256: "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
                },
                ModelFile {
                    repo: PARAKEET_REPO,
                    revision: PARAKEET_REV,
                    path: "decoder.int8.onnx",
                    file_name: "decoder.int8.onnx",
                    size_bytes: 11845275,
                    sha256: "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
                },
                ModelFile {
                    repo: PARAKEET_REPO,
                    revision: PARAKEET_REV,
                    path: "joiner.int8.onnx",
                    file_name: "joiner.int8.onnx",
                    size_bytes: 6355277,
                    sha256: "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
                },
                ModelFile {
                    repo: PARAKEET_REPO,
                    revision: PARAKEET_REV,
                    path: "tokens.txt",
                    file_name: "tokens.txt",
                    size_bytes: 93939,
                    sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                },
                ModelFile {
                    repo: "csukuangfj/vad",
                    revision: "fba88cd2e921609e7675c3aaf51e0b9b295da4bc",
                    path: "silero_vad.onnx",
                    file_name: "silero_vad.onnx",
                    size_bytes: 1807522,
                    sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
                },
            ],
        },
    ]
}
