// @author wzxgqtl 2026-11-21
// 产物登记：把电脑上的一个文件注册成「对象 + 产物」。
//
// 内容不进同步通道，只同步元数据；手机端按需通过 HTTP 拉取。
// 当前入口是命令行（--add-artifact），后续执行器产出文件时复用同一函数。

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    contract::{ArtifactInfo, ObjectRecord},
    store::{Blob, Store, now_millis},
};

/// 缩略图长边。列表页只拉缩略图，能省掉大量流量与解码开销。
const THUMB_MAX_EDGE: u32 = 320;

pub fn register_artifact(store: &Store, data_dir: &Path, source: &Path) -> Result<ObjectRecord> {
    if !source.is_file() {
        bail!("不是有效文件：{}", source.display());
    }

    let bytes = fs::read(source).with_context(|| format!("读取文件失败：{}", source.display()))?;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".to_owned());

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = format!("{:x}", hasher.finalize());

    let blob_id = Uuid::new_v4().to_string();
    let blob_dir = data_dir.join("blobs");
    fs::create_dir_all(&blob_dir).context("无法创建产物目录")?;

    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let stored = blob_dir.join(if extension.is_empty() {
        blob_id.clone()
    } else {
        format!("{blob_id}.{extension}")
    });
    fs::write(&stored, &bytes).context("写入产物失败")?;

    // 只有图片能本地生成缩略图；PDF 首页渲染需要额外依赖，留到后续再补。
    let thumb_path =
        make_thumbnail(&bytes, &blob_dir, &blob_id).map(|path| path.to_string_lossy().to_string());

    let mime = mime_for(&extension).to_owned();
    let blob = Blob {
        id: blob_id.clone(),
        sha256,
        size: bytes.len() as i64,
        mime: mime.clone(),
        name: name.clone(),
        path: stored.to_string_lossy().to_string(),
        thumb_path,
        created_at: now_millis(),
    };
    store.insert_blob(&blob)?;

    let info = ArtifactInfo {
        blob_id,
        name: name.clone(),
        mime,
        size: blob.size,
        sha256: blob.sha256.clone(),
        thumb_blob_id: None,
    };
    store.create_artifact_object(&info, &name)
}

fn make_thumbnail(bytes: &[u8], blob_dir: &Path, blob_id: &str) -> Option<PathBuf> {
    let image = image::load_from_memory(bytes).ok()?;
    let thumb = image.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE);
    let path = blob_dir.join(format!("{blob_id}.thumb.png"));
    thumb.save(&path).ok()?;
    Some(path)
}

fn mime_for(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "yaml" | "yml" => "text/yaml",
        _ => "application/octet-stream",
    }
}
