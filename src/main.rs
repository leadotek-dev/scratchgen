use anyhow::{Context, Result};
use clap::Parser;
use globwalk::GlobWalkerBuilder;
use image::{DynamicImage, GenericImageView, imageops::FilterType, RgbaImage};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(author, version, about = "3x3 Image Grid Generator", long_about = None)]
struct Cli {
    /// Config file (JSON or YAML)
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    pool: Vec<String>,
    #[arg(long, default_value_t = String::from("constrained"))]
    mode: String,
    #[arg(long, default_value_t = 256)]
    tile_size: u32,
    #[arg(long, default_value_t = String::from("cover"))]
    fit: String,
    #[arg(long, default_value_t = 3)]
    grid_size: usize,
    #[arg(long, default_value_t = 0)]
    seed: u64,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = String::from("#000000"))]
    background: String,
    #[arg(long, default_value_t = false)]
    allow_repeat_when_pool_small: bool,
    /// Generate example images into images/flowers and images/animals and exit
    #[arg(long, default_value_t = false)]
    generate_examples: bool,
}

#[derive(Deserialize, Debug)]
struct Config {
    pool: Option<Vec<String>>,
    weights: Option<HashMap<String, f64>>,
    min_count: Option<HashMap<String, i32>>,
    max_count: Option<HashMap<String, i32>>,
    mode: Option<String>,
    tile_size: Option<u32>,
    fit: Option<String>,
    seed: Option<u64>,
    output: Option<String>,
    background: Option<String>,
    grid_size: Option<usize>,
    allow_repeat_when_pool_small: Option<bool>,
}

#[derive(Debug, Clone)]
struct ImgMeta {
    path: PathBuf,
    weight: f64,
    min: i32,
    max: i32,
}

fn expand_pool(patterns: &[String]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let exts = ["jpg","jpeg","png","gif","webp","bmp","tiff"];
    for p in patterns {
        if Path::new(p).is_dir() {
            for entry in fs::read_dir(p)? {
                let e = entry?;
                let path = e.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                        if exts.contains(&ext.to_lowercase().as_str()) { files.push(path); }
                    }
                }
            }
        } else {
            let walker = GlobWalkerBuilder::from_patterns(".", &[p]).build()?;
            for entry in walker.filter_map(Result::ok) {
                let p = entry.path().to_path_buf();
                if p.is_file() { files.push(p); }
            }
        }
    }
    files.sort(); files.dedup();
    Ok(files)
}

fn build_images(config: &Config, pool_patterns: &[String]) -> Result<Vec<ImgMeta>> {
    let files = expand_pool(pool_patterns)?;
    if files.is_empty() { anyhow::bail!("No images found in pool"); }
    let weights = config.weights.as_ref();
    let mins = config.min_count.as_ref();
    let maxs = config.max_count.as_ref();
    let mut imgs = Vec::new();
    for f in files {
        let bn = f.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let mut w = 1.0;
        if let Some(ws) = weights {
            if let Some(v) = ws.get(&f.to_string_lossy().to_string()) { w = *v; }
            if let Some(v) = ws.get(&bn) { w = *v; }
        }
        let min = mins.and_then(|m| m.get(&f.to_string_lossy().to_string()).cloned()).or_else(|| mins.and_then(|m| m.get(&bn).cloned())).unwrap_or(0);
        let max = maxs.and_then(|m| m.get(&f.to_string_lossy().to_string()).cloned()).or_else(|| maxs.and_then(|m| m.get(&bn).cloned())).unwrap_or(i32::MAX);
        imgs.push(ImgMeta { path: f, weight: if w>0.0 { w } else { 0.0001 }, min, max });
    }
    Ok(imgs)
}

fn weighted_pick(images: &[ImgMeta], rng: &mut ChaCha8Rng) -> usize {
    let total: f64 = images.iter().map(|i| i.weight).sum();
    let mut r: f64 = rng.gen::<f64>() * total;
    for (idx, it) in images.iter().enumerate() {
        r -= it.weight;
        if r <= 0.0 { return idx; }
    }
    images.len() - 1
}

fn sample_without_replacement(images: &[ImgMeta], k: usize, rng: &mut ChaCha8Rng, allow_repeat: bool) -> Vec<PathBuf> {
    let n = images.len();
    if k > n && !allow_repeat { panic!("Pool has fewer than {} images and repeats are disallowed", k); }
    let mut copy: Vec<ImgMeta> = images.to_vec();
    let mut res = Vec::new();
    for _ in 0..k {
        if copy.is_empty() { if allow_repeat { copy = images.to_vec(); } else { break; } }
        let idx = weighted_pick(&copy, rng);
        res.push(copy[idx].path.clone());
        copy.remove(idx);
    }
    res
}

fn allocate_constrained(images: &[ImgMeta], total_tiles: usize, rng: &mut ChaCha8Rng) -> Vec<PathBuf> {
    let n = images.len();
    let total_weight: f64 = images.iter().map(|i| i.weight).sum();
    if total_weight <= 0.0 { panic!("Total weight <= 0"); }
    let mut alloc = vec![0i32; n];
    let mut remainders = vec![0f64; n];
    for i in 0..n {
        let expected = images[i].weight / total_weight * (total_tiles as f64);
        let base = expected.floor() as i32;
        alloc[i] = base;
        remainders[i] = expected - (base as f64);
    }
    let sum_base: i32 = alloc.iter().sum();
    let mut remaining = (total_tiles as i32) - sum_base;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a,&b| remainders[b].partial_cmp(&remainders[a]).unwrap());
    for i in 0..remaining as usize {
        alloc[order[i % n]] += 1;
    }
    // enforce min/max iteratively
    for _iter in 0..100 {
        let mut changed = false;
        // mins
        for i in 0..n {
            let min = images[i].min;
            if alloc[i] < min {
                let mut need = min - alloc[i];
                alloc[i] += need; changed = true;
                for j in 0..n {
                    if j==i { continue; }
                    let available = (alloc[j] - images[j].min).max(0);
                    if available<=0 { continue; }
                    let take = available.min(need);
                    alloc[j] -= take; need -= take;
                    if need==0 { break; }
                }
                if need>0 { panic!("Constraints infeasible: cannot satisfy min_count requirements"); }
            }
        }
        // maxs
        for i in 0..n {
            let max = images[i].max;
            if alloc[i] > max {
                let mut excess = alloc[i] - max;
                alloc[i] -= excess; changed = true;
                for j in 0..n {
                    if j==i { continue; }
                    let cap = (images[j].max - alloc[j]).max(0);
                    if cap<=0 { continue; }
                    let give = cap.min(excess);
                    alloc[j] += give; excess -= give;
                    if excess==0 { break; }
                }
                if excess>0 { panic!("Constraints infeasible: cannot redistribute excess from max_count"); }
            }
        }
        if !changed { break; }
    }
    // final adjust
    let sum: i32 = alloc.iter().sum();
    if sum != total_tiles as i32 {
        let mut diff = (total_tiles as i32) - sum;
        if diff > 0 {
            for &idx in &order {
                if alloc[idx] < images[idx].max {
                    alloc[idx] += 1; diff -= 1; if diff==0 { break; }
                }
            }
        } else if diff < 0 {
            diff = -diff;
            for &idx in order.iter().rev() {
                if alloc[idx] > images[idx].min {
                    alloc[idx] -= 1; diff -= 1; if diff==0 { break; }
                }
            }
        }
    }
    if alloc.iter().sum::<i32>() != total_tiles as i32 { panic!("Failed to allocate counts summing to {}", total_tiles); }
    let mut selected: Vec<PathBuf> = Vec::new();
    for i in 0..n {
        for _c in 0..alloc[i] { selected.push(images[i].path.clone()); }
    }
    // shuffle selected with rng
    selected.shuffle(rng);
    selected
}

fn sample_independent(images: &[ImgMeta], k: usize, rng: &mut ChaCha8Rng) -> Vec<PathBuf> {
    let mut res = Vec::new();
    for _ in 0..k { let idx = weighted_pick(images, rng); res.push(images[idx].path.clone()); }
    res
}

fn fit_image(img: DynamicImage, tile_size: u32, fit: &str, bg: image::Rgba<u8>) -> RgbaImage {
    match fit {
        "cover" => {
            let cropped = img.resize_to_fill(tile_size, tile_size, FilterType::Lanczos3);
            cropped.to_rgba8()
        }
        "contain" => {
            let resized = img.resize(tile_size, tile_size, FilterType::Lanczos3);
            let mut canvas = RgbaImage::from_pixel(tile_size, tile_size, bg);
            let (w,h) = (resized.width(), resized.height());
            let x = ((tile_size - w) / 2) as i64;
            let y = ((tile_size - h) / 2) as i64;
            image::imageops::overlay(&mut canvas, &resized.to_rgba8(), x, y);
            canvas
        }
        _ => {
            let resized = img.resize_exact(tile_size, tile_size, FilterType::Lanczos3);
            resized.to_rgba8()
        }
    }
}

fn compose_grid(paths: &[PathBuf], tile_size: u32, fit: &str, background: &str, output: &Path) -> Result<()> {
    // determine grid size from number of paths (assume square)
    let total = paths.len();
    let grid_n = (f64::from(total as u32).sqrt().round()) as u32;
    let cols = grid_n; let rows = grid_n; let w = tile_size * cols; let h = tile_size * rows;
    // parse background as hex #rrggbb
    let bg = if background.starts_with('#') && background.len()>=7 {
        let r = u8::from_str_radix(&background[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&background[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&background[5..7], 16).unwrap_or(0);
        image::Rgba([r,g,b,255])
    } else { image::Rgba([0,0,0,255]) };
    let mut canvas = RgbaImage::from_pixel(w, h, bg);
    for i in 0..total {
        let p = &paths[i];
        let img = image::open(p).with_context(|| format!("Failed to open image {:?}", p))?;
        let tile = fit_image(img, tile_size, fit, bg);
        let col = (i as u32 % cols) as i64;
        let row = (i as u32 / cols) as i64;
        let x = col * (tile_size as i64);
        let y = row * (tile_size as i64);
        image::imageops::overlay(&mut canvas, &tile, x, y);
    }
    canvas.save(output)?;
    Ok(())
}

fn generate_example_images() -> Result<()> {
    let flower_dir = Path::new("images/flowers");
    let animal_dir = Path::new("images/animals");
    fs::create_dir_all(flower_dir)?;
    fs::create_dir_all(animal_dir)?;
    // define examples (filename, color rgba)
    let flowers = vec![
        ("rose.jpg", [220u8,20,60,255]),
        ("daisy.jpg", [255,255,0,255]),
        ("tulip.jpg", [255,105,180,255]),
        ("orchid.jpg", [138,43,226,255])
    ];
    let animals = vec![
        ("lion.png", [218,165,32,255]),
        ("tiger.png", [255,140,0,255]),
        ("bear.png", [139,69,19,255]),
        ("wolf.png", [112,128,144,255])
    ];
    for (name, col) in flowers {
        let mut img = RgbaImage::from_pixel(400, 400, image::Rgba(col));
        let path = flower_dir.join(name);
        img.save(&path)?;
    }
    for (name, col) in animals {
        let mut img = RgbaImage::from_pixel(400, 400, image::Rgba(col));
        let path = animal_dir.join(name);
        img.save(&path)?;
    }
    println!("Wrote example images to images/flowers and images/animals");
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.generate_examples {
        generate_example_images()?;
        return Ok(());
    }
    // load config
    let mut config = Config{ pool: None, weights: None, min_count: None, max_count: None, mode: None, tile_size: None, fit: None, seed: None, output: None, background: None, grid_size: None, allow_repeat_when_pool_small: None };
    if let Some(cfg) = cli.config {
        let text = fs::read_to_string(&cfg)?;
        if cfg.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("json")).unwrap_or(false) {
            config = serde_json::from_str(&text)?;
        } else {
            config = serde_yaml::from_str(&text)?;
        }
    }
    // merge CLI overrides
    let pool_patterns: Vec<String> = if !cli.pool.is_empty() { cli.pool.clone() } else { config.pool.clone().unwrap_or_default() };
    let mode = if cli.mode != "constrained" { cli.mode.clone() } else { config.mode.clone().unwrap_or(cli.mode.clone()) };
    let tile_size = if cli.tile_size != 256 { cli.tile_size } else { config.tile_size.unwrap_or(cli.tile_size) };
    let fit = if cli.fit != "cover" { cli.fit.clone() } else { config.fit.clone().unwrap_or(cli.fit.clone()) };
    let seed = if cli.seed != 0 { cli.seed } else { config.seed.unwrap_or(cli.seed) };
    let output = if let Some(o) = cli.output { o } else if let Some(o2) = config.output.clone() { PathBuf::from(o2) } else { anyhow::bail!("--output required") };
    let background = if cli.background != "#000000" { cli.background.clone() } else { config.background.clone().unwrap_or(cli.background.clone()) };
    let allow_repeat = if cli.allow_repeat_when_pool_small { cli.allow_repeat_when_pool_small } else { config.allow_repeat_when_pool_small.unwrap_or(false) };

    let images = build_images(&config, &pool_patterns)?;
    let mut rng = if seed==0 { ChaCha8Rng::from_entropy() } else { ChaCha8Rng::seed_from_u64(seed) };
    let grid_n = config.grid_size.unwrap_or(cli.grid_size);
    let grid_n = if grid_n < 2 { 2usize } else { grid_n };
    let tiles_count = grid_n * grid_n;

    let selected: Vec<PathBuf> = match mode.as_str() {
        "independent" => sample_independent(&images, tiles_count, &mut rng),
        "without_replacement" => sample_without_replacement(&images, tiles_count, &mut rng, allow_repeat),
        _ => allocate_constrained(&images, tiles_count, &mut rng),
    };

    if let Some(parent) = output.parent() { fs::create_dir_all(parent)?; }
    compose_grid(&selected, tile_size, &fit, &background, &output)?;
    // print metadata
    let meta = serde_json::json!({"seed": seed, "mode": mode, "grid_size": grid_n, "tiles": selected.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(), "output": output.to_string_lossy()});
    println!("{}", serde_json::to_string_pretty(&meta)?);
    Ok(())
}
