use clap::Parser;
use std::path::{Path, PathBuf};
use std::io::{self, Write};
use rand::Rng; // 引入 rand::Rng trait

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;
// use symphonia::core::units::Time; // ✅ 修正：移除未使用的导入

// 引入 hound 库用于写入 WAV 文件
use hound::{WavWriter, SampleFormat as HoundSampleFormat};

/// 简单的多格式音频交错复制工具
///
/// 将一个音频文件（支持多种格式）复制为两份，并让它们的声音交错出现，每个交替块的时长是随机的。
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 输入的音频文件路径 (支持 .wav, .mp3, .flac, .m4a, .ogg 等)
    #[arg(short, long)]
    source: String,

    /// 输出文件的前缀 (例如: processed_audio 会生成 processed_audio_1.wav 和 processed_audio_2.wav)
    #[arg(short, long, default_value = "output_audio")]
    output_prefix: String,

    /// 每个随机交替块的最大持续时间 (秒). 实际时长将在 1 秒到此最大值之间随机选择。
    #[arg(short, long, default_value_t = 10)] // 默认最大为 10 秒
    max_chunk_duration: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let input_path = PathBuf::from(&args.source);
    let output_path1 = input_path.with_file_name(format!("{}_1.wav", args.output_prefix));
    let output_path2 = input_path.with_file_name(format!("{}_2.wav", args.output_prefix));
    let max_chunk_duration_seconds = args.max_chunk_duration;

    const MIN_CHUNK_DURATION_SECONDS: u64 = 1;

    if max_chunk_duration_seconds < MIN_CHUNK_DURATION_SECONDS {
        return Err(format!("错误：最大交替块时长 ({}) 不能小于最小交替块时长 ({})。",
                           max_chunk_duration_seconds, MIN_CHUNK_DURATION_SECONDS).into());
    }

    println!("正在处理音频文件：{}", input_path.display());
    println!("输出文件将是：{} 和 {}", output_path1.display(), output_path2.display());
    println!("每个交替块时长将在 {} 到 {} 秒之间随机选择。",
             MIN_CHUNK_DURATION_SECONDS, max_chunk_duration_seconds);

    if !input_path.exists() {
        return Err(format!("错误：输入文件不存在：{}", input_path.display()).into());
    }

    // 1. 打开媒体源
    let file = std::fs::File::open(&input_path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = input_path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &Default::default(), &Default::default())?;

    let mut format = probed.format;

    // 2. 选择音频轨道
    let track = format.tracks().iter()
        // ✅ 修正：检查 codec_params.codec 是否为 Some，而不是 CodecType::None
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "未找到音频轨道".to_string())?;

    let track_id = track.id;
    let codec_params = &track.codec_params;

    // 3. 创建解码器
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())?;

    let sample_rate = codec_params.sample_rate.ok_or("无法获取采样率")?;
    let channels = codec_params.channels.ok_or("无法获取声道数")?.count() as u16;

    // 假设输出是 16 位立体声 WAV
    let output_spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: HoundSampleFormat::Int,
    };

    let mut writer1 = WavWriter::create(&output_path1, output_spec.clone())?;
    let mut writer2 = WavWriter::create(&output_path2, output_spec.clone())?;

    let mut sample_buffer_converter: Option<SampleBuffer<i16>> = None; // 用于将不同格式的样本转换为 i16

    let mut current_chunk_idx = 0;
    let mut processed_samples_in_total: u64 = 0;
    let mut total_duration_frames: u64 = 0; // 声明为 mut

    // 估算总时长（帧数），用于进度显示。
    // Symphonia 的 CodecParameters 在不同版本可能字段不同。
    // 最可靠的方式是通过遍历所有包来累加解码帧的 duration，但这会慢很多。
    // 这里我们尝试使用 n_frames，如果不存在，则直接使用 1 进行进度条除法，并打印警告。
    if let Some(n_frames) = track.codec_params.n_frames {
        total_duration_frames = n_frames;
    } else {
        println!("警告：无法获取精确的总帧数，进度显示可能不准确。");
        // 为确保除数不为零，给一个很大的默认值，或者直接禁用进度条
        total_duration_frames = 1; // 避免除零错误
    }


    let mut rng = rand::thread_rng();

    'main_loop: loop {
        // 随机生成当前块的持续时间
        let current_chunk_duration_seconds = rng.gen_range(MIN_CHUNK_DURATION_SECONDS..=max_chunk_duration_seconds);
        // 计算当前块所需的样本数 (所有声道)
        // 注意：这里是目标样本数，实际读取可能不足
        let target_samples_for_this_chunk = sample_rate as usize * current_chunk_duration_seconds as usize * channels as usize;

        let mut samples_read_in_this_chunk = 0;
        let mut chunk_samples: Vec<i16> = Vec::with_capacity(target_samples_for_this_chunk);
        let mut end_of_file = false;

        // 循环读取数据包并解码，直到收集到足够当前块的样本
        'decode_loop: loop {
            // 如果已经收集到足够样本，且还有数据未写入，则跳出解码循环
            if samples_read_in_this_chunk >= target_samples_for_this_chunk {
                break 'decode_loop;
            }

            match format.next_packet() {
                Ok(packet) => {
                    // 如果不是当前音频轨道的包，跳过
                    if packet.track_id() != track_id {
                        continue;
                    }

                    // 解码数据包
                    match decoder.decode(&packet) {
                        Ok(decoded_frame) => {
                            // 如果样本转换器还没有初始化，根据解码帧的格式进行初始化
                            if sample_buffer_converter.is_none() {
                                let spec = decoded_frame.spec();
                                let capacity = decoded_frame.capacity(); // 使用 capacity 而非 duration
                                // ✅ 修正：对 spec 解引用
                                sample_buffer_converter = Some(SampleBuffer::<i16>::new(capacity as u64, *spec));
                            }

                            // 转换样本到 i16
                            if let Some(converter) = &mut sample_buffer_converter {
                                // ✅ 修正：使用 copy_interleaved_ref 方法，并移除 `?`
                                converter.copy_interleaved_ref(decoded_frame); // <-- 移除 ?
                                let samples = converter.samples();

                                // 将样本添加到当前块
                                for &s in samples {
                                    if samples_read_in_this_chunk < target_samples_for_this_chunk {
                                        chunk_samples.push(s);
                                        samples_read_in_this_chunk += 1;
                                    } else {
                                        // 已经收集到足够样本，将剩余样本放回缓冲区（如果有）
                                        // Symphonia 没有内置的回溯机制，这里直接跳出
                                        // 这里的逻辑会丢弃当前帧中超出目标chunk_samples容量的部分
                                        break;
                                    }
                                }
                                // 如果这一帧的解码使得我们收集了足够多的样本，就跳出解码循环
                                if samples_read_in_this_chunk >= target_samples_for_this_chunk {
                                    break 'decode_loop;
                                }
                            }
                        }
                        Err(SymphoniaError::DecodeError(_)) => {
                            // 解码错误，可能是损坏的数据，跳过
                            continue;
                        }
                        Err(err) => return Err(format!("解码时发生错误：{}", err).into()),
                    }
                }
                Err(SymphoniaError::ResetRequired) => {
                    // 解码器需要重置，通常发生在 Seek 操作后，但这里我们不 Seek
                    decoder.reset();
                }
                Err(SymphoniaError::IoError(err)) => {
                    // IO 错误，可能是文件结束
                    if err.kind() == io::ErrorKind::UnexpectedEof {
                        end_of_file = true;
                        break 'decode_loop; // 退出解码循环
                    }
                    return Err(format!("IO 错误：{}", err).into());
                }
                Err(err) => return Err(format!("读取数据包时发生错误：{}", err).into()),
            }
        }

        // 如果当前块没有读取到任何样本，并且已经到达文件末尾，则退出主循环
        if samples_read_in_this_chunk == 0 && end_of_file {
            break 'main_loop;
        }

        // 创建一个与当前块相同大小的静音块
        let silence_chunk: Vec<i16> = vec![0; samples_read_in_this_chunk];

        // 根据当前块索引决定写入哪个文件有声音，哪个文件是静音
        if current_chunk_idx % 2 == 0 {
            // 偶数块：文件1有声音，文件2静音
            for &sample in &chunk_samples {
                writer1.write_sample(sample)?;
            }
            for &sample in &silence_chunk {
                writer2.write_sample(sample)?;
            }
        } else {
            // 奇数块：文件1静音，文件2有声音
            for &sample in &silence_chunk {
                writer1.write_sample(sample)?;
            }
            for &sample in &chunk_samples {
                writer2.write_sample(sample)?;
            }
        }

        current_chunk_idx += 1;
        processed_samples_in_total += samples_read_in_this_chunk as u64;

        // 打印进度
        let progress_percent = (processed_samples_in_total as f64 / total_duration_frames as f64) * 100.0;
        print!("\r正在处理... {:.2}%", progress_percent.min(100.0)); // 确保不超过100%
        io::stdout().flush()?;

        // 如果在读取当前块时到达了文件末尾，并且当前块的样本数小于目标样本数，
        // 则说明文件已完全读取，处理完当前剩下的样本后退出主循环。
        if end_of_file && samples_read_in_this_chunk < target_samples_for_this_chunk {
            break 'main_loop;
        }
    }

    writer1.flush()?;
    writer2.flush()?;

    println!("\n处理完成！");
    println!("输出文件：{} 和 {}", output_path1.display(), output_path2.display());

    Ok(())
}