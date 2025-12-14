use anyhow::{anyhow, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::vit;
use hf_hub::{api::sync::Api, Repo, RepoType};
use image::{ImageBuffer, Rgba};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use super::{ContentFilter, Frame};

const MODEL_REPO: &str = "LukeJacob2023/nsfw-image-detector";
const IMAGE_SIZE: usize = 224;
const NSFW_THRESHOLD: f32 = 0.05; // Very aggressive - block anything with >5% NSFW probability

// Class indices from model config:
// 0: drawings (safe)
// 1: hentai (BLOCK)
// 2: neutral (safe)
// 3: porn (BLOCK)
// 4: sexy (BLOCK)
const CLASS_DRAWINGS: usize = 0;
const CLASS_HENTAI: usize = 1;
const CLASS_NEUTRAL: usize = 2;
const CLASS_PORN: usize = 3;
const CLASS_SEXY: usize = 4;

/// NSFW filter using LukeJacob2023/nsfw-image-detector ViT model
/// 5-class model: drawings (safe), hentai (block), neutral (safe), porn (block), sexy (block)
pub struct NsfwFilter {
    model: Mutex<vit::Model>,
    device: Device,
}

impl NsfwFilter {
    pub fn new() -> Result<Self> {
        #[cfg(feature = "metal")]
        let device = Device::new_metal(0).unwrap_or(Device::Cpu);
        #[cfg(not(feature = "metal"))]
        let device = Device::Cpu;

        log::info!("Loading NSFW detection model on {:?}", device);

        let api = Api::new()?;
        let repo = api.repo(Repo::new(MODEL_REPO.to_string(), RepoType::Model));

        let model_path = repo.get("model.safetensors")?;
        let config_path = repo.get("config.json")?;

        let config: vit::Config = serde_json::from_str(&std::fs::read_to_string(config_path)?)?;
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)? };
        let model = vit::Model::new(&config, 5, vb)?; // 5 classes: drawings, hentai, neutral, porn, sexy

        log::info!("NSFW model loaded successfully");

        Ok(Self {
            model: Mutex::new(model),
            device,
        })
    }

    fn preprocess_batch(&self, scaled_images: &[Vec<u8>]) -> Result<Tensor> {
        // LukeJacob2023/nsfw-image-detector uses mean=0.5, std=0.5 for all channels
        let mean = 0.5;
        let std = 0.5;
        let batch_size = scaled_images.len();

        let mut data = vec![0f32; batch_size * 3 * IMAGE_SIZE * IMAGE_SIZE];

        for (batch_idx, scaled_rgb) in scaled_images.iter().enumerate() {
            let offset = batch_idx * 3 * IMAGE_SIZE * IMAGE_SIZE;
            for i in 0..(IMAGE_SIZE * IMAGE_SIZE) {
                let r = scaled_rgb[i * 3] as f32 / 255.0;
                let g = scaled_rgb[i * 3 + 1] as f32 / 255.0;
                let b = scaled_rgb[i * 3 + 2] as f32 / 255.0;

                // CHW format with normalization
                data[offset + i] = (r - mean) / std;
                data[offset + IMAGE_SIZE * IMAGE_SIZE + i] = (g - mean) / std;
                data[offset + 2 * IMAGE_SIZE * IMAGE_SIZE + i] = (b - mean) / std;
            }
        }

        let tensor = Tensor::from_vec(data, (batch_size, 3, IMAGE_SIZE, IMAGE_SIZE), &self.device)?;
        Ok(tensor)
    }

    /// Sample frames from video using ffmpeg
    fn sample_video(&self, path: &Path, interval_secs: u32) -> Result<Vec<Frame>> {
        let temp_dir = std::env::temp_dir().join("cleo-frames");
        std::fs::create_dir_all(&temp_dir)?;

        // Clean up any existing frames
        for entry in std::fs::read_dir(&temp_dir)? {
            if let Ok(entry) = entry {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        // Extract frames at interval using ffmpeg
        // -vf fps=1/N extracts 1 frame every N seconds
        let output = Command::new("ffmpeg")
            .args([
                "-i",
                path.to_str().ok_or_else(|| anyhow!("Invalid path"))?,
                "-vf",
                &format!("fps=1/{}", interval_secs),
                "-f",
                "image2",
                temp_dir.join("frame_%04d.png").to_str().unwrap(),
            ])
            .output();

        match output {
            Ok(result) if result.status.success() => {}
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                log::warn!("ffmpeg failed: {}", stderr);
                return Ok(vec![]);
            }
            Err(e) => {
                log::warn!("ffmpeg not available: {}", e);
                return Ok(vec![]);
            }
        }

        // Load extracted frames
        let mut frames = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&temp_dir)?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let frame_path = entry.path();
            if frame_path.extension().is_some_and(|e| e == "png") {
                match image::open(&frame_path) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (width, height) = rgba.dimensions();
                        frames.push(Frame {
                            rgba: rgba.into_raw(),
                            width,
                            height,
                        });
                    }
                    Err(e) => log::warn!("Failed to load frame {:?}: {}", frame_path, e),
                }
                let _ = std::fs::remove_file(frame_path);
            }
        }

        log::info!("Extracted {} frames from video", frames.len());
        Ok(frames)
    }
}

impl ContentFilter for NsfwFilter {
    fn sample(&self, path: &Path, interval_secs: u32) -> Result<Vec<Frame>> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" => {
                let img = image::open(path)?;
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();

                Ok(vec![Frame {
                    rgba: rgba.into_raw(),
                    width,
                    height,
                }])
            }
            "mp4" | "mov" | "webm" | "mkv" => {
                self.sample_video(path, interval_secs)
            }
            _ => Err(anyhow!("Unsupported file type: {}", ext)),
        }
    }

    fn scale(&self, rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(width, height, rgba.to_vec())
                .ok_or_else(|| anyhow!("Invalid image dimensions"))?;

        let resized = image::imageops::resize(
            &img,
            IMAGE_SIZE as u32,
            IMAGE_SIZE as u32,
            image::imageops::FilterType::Triangle,
        );

        // Convert RGBA to RGB
        let mut rgb = Vec::with_capacity(IMAGE_SIZE * IMAGE_SIZE * 3);
        for pixel in resized.pixels() {
            rgb.push(pixel[0]); // R
            rgb.push(pixel[1]); // G
            rgb.push(pixel[2]); // B
        }

        Ok(rgb)
    }

    fn classify(&self, scaled_images: &[Vec<u8>]) -> Result<Vec<bool>> {
        if scaled_images.is_empty() {
            return Ok(vec![]);
        }

        // Validate all images
        eprintln!("[DEBUG] classify: validating {} images", scaled_images.len());
        for (i, img) in scaled_images.iter().enumerate() {
            if img.len() != IMAGE_SIZE * IMAGE_SIZE * 3 {
                return Err(anyhow!(
                    "Image {} expected {}x{}x3 RGB, got {} bytes",
                    i, IMAGE_SIZE, IMAGE_SIZE, img.len()
                ));
            }
        }

        let batch_size = scaled_images.len();
        eprintln!("[DEBUG] classify: preprocessing batch");
        let input = self.preprocess_batch(scaled_images)?;
        eprintln!("[DEBUG] classify: acquiring model lock");
        let model = self.model.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        eprintln!("[DEBUG] classify: running forward pass on {} images...", batch_size);
        let logits = model.forward(&input)?;
        eprintln!("[DEBUG] classify: forward pass complete");

        // Softmax to get probabilities - shape is (batch_size, 5)
        let probs = candle_nn::ops::softmax(&logits, 1)?;
        let probs_vec: Vec<f32> = probs.flatten_all()?.to_vec1()?;

        let mut results = Vec::with_capacity(batch_size);

        for batch_idx in 0..batch_size {
            let offset = batch_idx * 5;
            let drawings_prob = probs_vec.get(offset + CLASS_DRAWINGS).copied().unwrap_or(0.0);
            let hentai_prob = probs_vec.get(offset + CLASS_HENTAI).copied().unwrap_or(0.0);
            let neutral_prob = probs_vec.get(offset + CLASS_NEUTRAL).copied().unwrap_or(0.0);
            let porn_prob = probs_vec.get(offset + CLASS_PORN).copied().unwrap_or(0.0);
            let sexy_prob = probs_vec.get(offset + CLASS_SEXY).copied().unwrap_or(0.0);

            let nsfw_prob = hentai_prob + porn_prob + sexy_prob;

            let blocked_class = if hentai_prob >= NSFW_THRESHOLD {
                Some(("hentai", hentai_prob))
            } else if porn_prob >= NSFW_THRESHOLD {
                Some(("porn", porn_prob))
            } else if sexy_prob >= NSFW_THRESHOLD {
                Some(("sexy", sexy_prob))
            } else {
                None
            };

            let is_safe = blocked_class.is_none();

            if is_safe {
                log::info!(
                    "[NSFW] #{} SAFE - nsfw: {:.1}% (drawings={:.1}%, neutral={:.1}%)",
                    batch_idx, nsfw_prob * 100.0, drawings_prob * 100.0, neutral_prob * 100.0
                );
            } else {
                let (class_name, class_prob) = blocked_class.unwrap();
                log::warn!(
                    "[NSFW] #{} BLOCKED - {} at {:.1}%",
                    batch_idx, class_name, class_prob * 100.0
                );
            }

            results.push(is_safe);
        }

        log::info!("[NSFW] Batch of {} classified in single forward pass", batch_size);
        Ok(results)
    }
}
