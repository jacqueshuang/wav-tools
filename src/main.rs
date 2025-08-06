// use clap::Parser;
// use std::path::{Path, PathBuf};
// use std::io::{self, Write};
// use rand::Rng;
//
// use symphonia::core::audio::{SampleBuffer, SignalSpec};
// use symphonia::core::codecs::{CodecType, DecoderOptions, CODEC_TYPE_NULL};
// use symphonia::core::errors::Error as SymphoniaError;
// use symphonia::core::io::MediaSourceStream;
// use symphonia::core::probe::Hint;
//
// use hound::{WavWriter, SampleFormat as HoundSampleFormat};
//
// /// 简单的多格式音频交错复制工具
// ///
// /// 将一个音频文件（支持多种格式）复制为两份，并让它们的声音交错出现，每个交替块的时长是随机的。
// #[derive(Parser, Debug)]
// #[command(author, version, about, long_about = None)]
// struct Args {
//     /// 输入的音频文件路径 (支持 .wav, .mp3, .flac, .m4a, .ogg 等)
//     #[arg(short, long)]
//     source: String,
//
//     /// 输出文件的前缀 (例如: processed_audio 会生成 processed_audio_1.wav 和 processed_audio_2.wav)
//     #[arg(short, long, default_value = "output_audio")]
//     output_prefix: String,
//
//     /// 每个随机交替块的最大持续时间 (秒). 实际时长将在 1 秒到此最大值之间随机选择。
//     #[arg(short, long, default_value_t = 5)] // 默认最大为 5 秒
//     max_chunk_duration: u64,
// }
//
// fn main() -> Result<(), Box<dyn std::error::Error>> {
//     let args = Args::parse();
//
//     let input_path = PathBuf::from(&args.source);
//     let output_path1 = input_path.with_file_name(format!("{}_1.wav", args.output_prefix));
//     let output_path2 = input_path.with_file_name(format!("{}_2.wav", args.output_prefix));
//     let max_chunk_duration_seconds = args.max_chunk_duration;
//
//     const MIN_CHUNK_DURATION_SECONDS: u64 = 1;
//
//     if max_chunk_duration_seconds < MIN_CHUNK_DURATION_SECONDS {
//         return Err(format!("错误：最大交替块时长 ({}) 不能小于最小交替块时长 ({})。",
//                            max_chunk_duration_seconds, MIN_CHUNK_DURATION_SECONDS).into());
//     }
//
//     println!("正在处理音频文件：{}", input_path.display());
//     println!("输出文件将是：{} 和 {}", output_path1.display(), output_path2.display());
//     println!("每个交替块时长将在 {} 到 {} 秒之间随机选择。",
//              MIN_CHUNK_DURATION_SECONDS, max_chunk_duration_seconds);
//
//     if !input_path.exists() {
//         return Err(format!("错误：输入文件不存在：{}", input_path.display()).into());
//     }
//
//     // 1. 打开媒体源
//     let file = std::fs::File::open(&input_path)?;
//     let mss = MediaSourceStream::new(Box::new(file), Default::default());
//
//     let mut hint = Hint::new();
//     if let Some(extension) = input_path.extension().and_then(|s| s.to_str()) {
//         hint.with_extension(extension);
//     }
//
//     let probed = symphonia::default::get_probe()
//         .format(&hint, mss, &Default::default(), &Default::default())?;
//
//     let mut format = probed.format;
//
//     // 2. 选择音频轨道
//     let track = format.tracks().iter()
//         .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
//         .ok_or_else(|| "未找到音频轨道".to_string())?;
//
//     let track_id = track.id;
//     let codec_params = &track.codec_params;
//
//     // 3. 创建解码器
//     let mut decoder = symphonia::default::get_codecs()
//         .make(&codec_params, &DecoderOptions::default())?;
//
//     let sample_rate = codec_params.sample_rate.ok_or("无法获取采样率")?;
//     let channels = codec_params.channels.ok_or("无法获取声道数")?.count() as u16;
//
//     // 假设输出是 16 位立体声 WAV
//     let output_spec = hound::WavSpec {
//         channels,
//         sample_rate,
//         bits_per_sample: 16,
//         sample_format: HoundSampleFormat::Int,
//     };
//
//     let mut writer1 = WavWriter::create(&output_path1, output_spec.clone())?;
//     let mut writer2 = WavWriter::create(&output_path2, output_spec.clone())?;
//
//     let mut sample_buffer_converter: Option<SampleBuffer<i16>> = None; // 用于将不同格式的样本转换为 i16
//
//     let mut current_chunk_idx = 0;
//     let mut processed_frames_for_progress: u64 = 0; // 追踪已处理的每声道帧数，用于进度条
//     let mut total_duration_frames: u64 = 0; // 总的每声道帧数 (Symphonia的n_frames)
//
//     // 估算总时长（帧数），用于进度显示。
//     if let Some(n_frames) = track.codec_params.n_frames {
//         total_duration_frames = n_frames;
//     } else {
//         println!("警告：无法获取精确的总帧数，进度显示可能不准确。");
//         total_duration_frames = 1; // 避免除零错误
//     }
//
//     let mut rng = rand::thread_rng();
//
//     // 用于暂存从解码帧中读取但未完全放入当前 chunk 的样本
//     let mut leftover_samples: Vec<i16> = Vec::new();
//
//     'main_loop: loop {
//         let current_chunk_duration_seconds = rng.gen_range(MIN_CHUNK_DURATION_SECONDS..=max_chunk_duration_seconds);
//
//         // 计算当前块所需的总交错样本数
//         let target_interleaved_samples_for_this_chunk = (sample_rate as u64 * current_chunk_duration_seconds * channels as u64) as usize;
//
//         // ✅ 修正：确保 current_chunk_samples 只包含当前块的样本，并且精确控制其大小
//         let mut current_chunk_samples: Vec<i16> = Vec::with_capacity(target_interleaved_samples_for_this_chunk);
//         let mut samples_collected_in_current_chunk = 0;
//         let mut end_of_file = false;
//
//         // 首先处理剩余样本
//         let num_from_leftover = (target_interleaved_samples_for_this_chunk - samples_collected_in_current_chunk).min(leftover_samples.len());
//         current_chunk_samples.extend_from_slice(&leftover_samples[..num_from_leftover]);
//         samples_collected_in_current_chunk += num_from_leftover;
//         leftover_samples.drain(..num_from_leftover); // 移除已使用的剩余样本
//
//         // 循环读取数据包并解码，直到收集到足够当前块的样本
//         'decode_loop: loop {
//             if samples_collected_in_current_chunk >= target_interleaved_samples_for_this_chunk {
//                 break 'decode_loop;
//             }
//
//             match format.next_packet() {
//                 Ok(packet) => {
//                     if packet.track_id() != track_id {
//                         continue;
//                     }
//
//                     match decoder.decode(&packet) {
//                         Ok(decoded_frame) => {
//                             if sample_buffer_converter.is_none() {
//                                 let spec = decoded_frame.spec();
//                                 sample_buffer_converter = Some(SampleBuffer::<i16>::new(decoded_frame.capacity() as u64, *spec));
//                             }
//
//                             if let Some(converter) = &mut sample_buffer_converter {
//                                 converter.copy_interleaved_ref(decoded_frame);
//                                 let samples_from_frame = converter.samples(); // 这是该帧的所有交错样本
//
//                                 // 计算还需要多少样本来填满当前 chunk
//                                 let remaining_in_chunk = target_interleaved_samples_for_this_chunk - samples_collected_in_current_chunk;
//                                 // 从当前帧中取多少样本
//                                 let to_take_from_frame = remaining_in_chunk.min(samples_from_frame.len());
//
//                                 current_chunk_samples.extend_from_slice(&samples_from_frame[..to_take_from_frame]);
//                                 samples_collected_in_current_chunk += to_take_from_frame;
//
//                                 // 如果当前帧还有剩余样本，保存到 leftover_samples
//                                 if to_take_from_frame < samples_from_frame.len() {
//                                     leftover_samples.extend_from_slice(&samples_from_frame[to_take_from_frame..]);
//                                 }
//
//                                 if samples_collected_in_current_chunk >= target_interleaved_samples_for_this_chunk {
//                                     break 'decode_loop;
//                                 }
//                             }
//                         }
//                         Err(SymphoniaError::DecodeError(_)) => {
//                             continue;
//                         }
//                         Err(err) => return Err(format!("解码时发生错误：{}", err).into()),
//                     }
//                 }
//                 Err(SymphoniaError::ResetRequired) => {
//                     decoder.reset();
//                 }
//                 Err(SymphoniaError::IoError(err)) => {
//                     if err.kind() == io::ErrorKind::UnexpectedEof {
//                         end_of_file = true;
//                         break 'decode_loop;
//                     }
//                     return Err(format!("IO 错误：{}", err).into());
//                 }
//                 Err(err) => return Err(format!("读取数据包时发生错误：{}", err).into()),
//             }
//         }
//
//         // 如果当前块没有读取到任何样本，并且已经到达文件末尾，则退出主循环
//         if samples_collected_in_current_chunk == 0 && end_of_file {
//             break 'main_loop;
//         }
//
//         // 创建一个与当前块相同大小的静音块
//         let silence_chunk: Vec<i16> = vec![0; samples_collected_in_current_chunk];
//
//         // 根据当前块索引决定写入哪个文件有声音，哪个文件是静音
//         if current_chunk_idx % 2 == 0 {
//             // 偶数块：文件1有声音，文件2静音
//             for &sample in &current_chunk_samples {
//                 writer1.write_sample(sample)?;
//             }
//             for &sample in &silence_chunk {
//                 writer2.write_sample(sample)?;
//             }
//         } else {
//             // 奇数块：文件1静音，文件2有声音
//             for &sample in &silence_chunk {
//                 writer1.write_sample(sample)?;
//             }
//             for &sample in &current_chunk_samples {
//                 writer2.write_sample(sample)?;
//             }
//         }
//
//         current_chunk_idx += 1;
//         // ✅ 修正：进度条根据处理的总帧数来计算
//         // samples_collected_in_current_chunk 是当前块的交错样本数
//         // 转换为每声道帧数：samples_collected_in_current_chunk / channels
//         processed_frames_for_progress += (samples_collected_in_current_chunk as u64) / (channels as u64);
//
//         // 打印进度
//         let progress_percent = (processed_frames_for_progress as f64 / total_duration_frames as f64) * 100.0;
//         print!("\r正在处理... {:.2}%", progress_percent.min(100.0)); // 确保不超过100%
//         io::stdout().flush()?;
//
//         // 如果在读取当前块时到达了文件末尾，并且当前块的样本数小于目标样本数，
//         // 则说明文件已完全读取，处理完当前剩下的样本后退出主循环。
//         if end_of_file && samples_collected_in_current_chunk < target_interleaved_samples_for_this_chunk {
//             break 'main_loop;
//         }
//     }
//
//     writer1.flush()?;
//     writer2.flush()?;
//
//     println!("\n处理完成！");
//     println!("输出文件：{} 和 {}", output_path1.display(), output_path2.display());
//
//     Ok(())
// }


use clap::Parser;
use std::path::{Path, PathBuf};
use std::io::{self, Write};
use rand::Rng;

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;

use hound::{WavWriter, SampleFormat as HoundSampleFormat};

/// 简单的多格式音频交错复制工具
///
/// 将一个音频文件（支持多种格式）复制为两份，并让它们的声音交错出现，每个交替块的时长和响度是随机的。
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
    #[arg(short, long, default_value_t = 5)] // 默认最大为 5 秒
    max_chunk_duration: u64,

    /// 随机响度的最小倍数。实际响度将在该值和最大值之间随机选择。
    #[arg(long, default_value_t = 1.0)]
    min_volume_multiplier: f32,

    /// 随机响度的最大倍数。实际响度将在最小值和该值之间随机选择。
    /// 响度倍数为 1.0 意味着不改变音量。
    #[arg(long, default_value_t = 1.0)]
    max_volume_multiplier: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let input_path = PathBuf::from(&args.source);
    let output_path1 = input_path.with_file_name(format!("{}_1.wav", args.output_prefix));
    let output_path2 = input_path.with_file_name(format!("{}_2.wav", args.output_prefix));
    let max_chunk_duration_seconds = args.max_chunk_duration;
    let min_volume_multiplier = args.min_volume_multiplier;
    let max_volume_multiplier = args.max_volume_multiplier;

    const MIN_CHUNK_DURATION_SECONDS: u64 = 1;

    if max_chunk_duration_seconds < MIN_CHUNK_DURATION_SECONDS {
        return Err(format!("错误：最大交替块时长 ({}) 不能小于最小交替块时长 ({})。",
                           max_chunk_duration_seconds, MIN_CHUNK_DURATION_SECONDS).into());
    }

    if min_volume_multiplier < 0.0 {
        return Err("错误：最小响度倍数不能为负值。".into());
    }

    if max_volume_multiplier < min_volume_multiplier {
        return Err(format!("错误：最大响度倍数 ({}) 不能小于最小响度倍数 ({})。",
                           max_volume_multiplier, min_volume_multiplier).into());
    }

    println!("正在处理音频文件：{}", input_path.display());
    println!("输出文件将是：{} 和 {}", output_path1.display(), output_path2.display());
    println!("每个交替块时长将在 {} 到 {} 秒之间随机选择。",
             MIN_CHUNK_DURATION_SECONDS, max_chunk_duration_seconds);
    println!("每个交替块的响度倍数将在 {} 到 {} 之间随机选择。",
             min_volume_multiplier, max_volume_multiplier);

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
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "未找到音频轨道".to_string())?;

    let track_id = track.id;
    let codec_params = &track.codec_params;

    // 3. 创建解码器
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())?;

    let sample_rate = codec_params.sample_rate.ok_or("无法获取采样率")?;
    let channels = codec_params.channels.ok_or("无法获取声道数")?.count() as u16;

    let output_spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: HoundSampleFormat::Int,
    };

    let mut writer1 = WavWriter::create(&output_path1, output_spec.clone())?;
    let mut writer2 = WavWriter::create(&output_path2, output_spec.clone())?;

    let mut sample_buffer_converter: Option<SampleBuffer<i16>> = None;

    let mut current_chunk_idx = 0;
    let mut processed_frames_for_progress: u64 = 0;
    let mut total_duration_frames: u64 = 0;

    if let Some(n_frames) = track.codec_params.n_frames {
        total_duration_frames = n_frames;
    } else {
        println!("警告：无法获取精确的总帧数，进度显示可能不准确。");
        total_duration_frames = 1;
    }

    let mut rng = rand::thread_rng();
    let mut leftover_samples: Vec<i16> = Vec::new();

    'main_loop: loop {
        let current_chunk_duration_seconds = rng.gen_range(MIN_CHUNK_DURATION_SECONDS..=max_chunk_duration_seconds);
        let volume_multiplier = rng.gen_range(min_volume_multiplier..=max_volume_multiplier);

        let target_interleaved_samples_for_this_chunk = (sample_rate as u64 * current_chunk_duration_seconds * channels as u64) as usize;

        let mut current_chunk_samples: Vec<i16> = Vec::with_capacity(target_interleaved_samples_for_this_chunk);
        let mut samples_collected_in_current_chunk = 0;
        let mut end_of_file = false;

        let num_from_leftover = (target_interleaved_samples_for_this_chunk - samples_collected_in_current_chunk).min(leftover_samples.len());
        current_chunk_samples.extend_from_slice(&leftover_samples[..num_from_leftover]);
        samples_collected_in_current_chunk += num_from_leftover;
        leftover_samples.drain(..num_from_leftover);

        'decode_loop: loop {
            if samples_collected_in_current_chunk >= target_interleaved_samples_for_this_chunk {
                break 'decode_loop;
            }

            match format.next_packet() {
                Ok(packet) => {
                    if packet.track_id() != track_id {
                        continue;
                    }

                    match decoder.decode(&packet) {
                        Ok(decoded_frame) => {
                            if sample_buffer_converter.is_none() {
                                let spec = decoded_frame.spec();
                                sample_buffer_converter = Some(SampleBuffer::<i16>::new(decoded_frame.capacity() as u64, *spec));
                            }

                            if let Some(converter) = &mut sample_buffer_converter {
                                converter.copy_interleaved_ref(decoded_frame);
                                let samples_from_frame = converter.samples();

                                let remaining_in_chunk = target_interleaved_samples_for_this_chunk - samples_collected_in_current_chunk;
                                let to_take_from_frame = remaining_in_chunk.min(samples_from_frame.len());

                                current_chunk_samples.extend_from_slice(&samples_from_frame[..to_take_from_frame]);
                                samples_collected_in_current_chunk += to_take_from_frame;

                                if to_take_from_frame < samples_from_frame.len() {
                                    leftover_samples.extend_from_slice(&samples_from_frame[to_take_from_frame..]);
                                }

                                if samples_collected_in_current_chunk >= target_interleaved_samples_for_this_chunk {
                                    break 'decode_loop;
                                }
                            }
                        }
                        Err(SymphoniaError::DecodeError(_)) => {
                            continue;
                        }
                        Err(err) => return Err(format!("解码时发生错误：{}", err).into()),
                    }
                }
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                }
                Err(SymphoniaError::IoError(err)) => {
                    if err.kind() == io::ErrorKind::UnexpectedEof {
                        end_of_file = true;
                        break 'decode_loop;
                    }
                    return Err(format!("IO 错误：{}", err).into());
                }
                Err(err) => return Err(format!("读取数据包时发生错误：{}", err).into()),
            }
        }

        if samples_collected_in_current_chunk == 0 && end_of_file {
            break 'main_loop;
        }

        let silence_chunk: Vec<i16> = vec![0; samples_collected_in_current_chunk];
        let adjusted_chunk_samples: Vec<i16> = current_chunk_samples.iter().map(|&s| {
            (s as f32 * volume_multiplier).clamp(i16::MIN as f32, i16::MAX as f32) as i16
        }).collect();

        if current_chunk_idx % 2 == 0 {
            for &sample in &adjusted_chunk_samples {
                writer1.write_sample(sample)?;
            }
            for &sample in &silence_chunk {
                writer2.write_sample(sample)?;
            }
        } else {
            for &sample in &silence_chunk {
                writer1.write_sample(sample)?;
            }
            for &sample in &adjusted_chunk_samples {
                writer2.write_sample(sample)?;
            }
        }

        current_chunk_idx += 1;
        processed_frames_for_progress += (samples_collected_in_current_chunk as u64) / (channels as u64);

        let progress_percent = (processed_frames_for_progress as f64 / total_duration_frames as f64) * 100.0;
        print!("\r正在处理... {:.2}%", progress_percent.min(100.0));
        io::stdout().flush()?;

        if end_of_file && samples_collected_in_current_chunk < target_interleaved_samples_for_this_chunk {
            break 'main_loop;
        }
    }

    writer1.flush()?;
    writer2.flush()?;

    println!("\n处理完成！");
    println!("输出文件：{} 和 {}", output_path1.display(), output_path2.display());

    Ok(())
}