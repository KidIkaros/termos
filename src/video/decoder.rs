/// Frame decoder: spawns `ffmpeg` to decode video files to raw RGB24 frames.
///
/// Uses ffmpeg's CLI rather than C bindings — no dev libraries needed.
/// Decoding runs on a dedicated OS thread so frames are ready when the
/// render path asks for them.
use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;


/// Metadata about a video file.
#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_secs: f64,
}

/// A decoded RGB24 frame.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Flat `[R, G, B, R, G, B, ...]` bytes.
    pub rgb: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Presentation timestamp in seconds.
    pub pts_secs: f64,
}

/// Commands sent to the decoder thread.
enum DecoderCommand {
    /// Start decoding from the given source.
    Decode { source: String, target_w: u32, target_h: u32, fps: u32 },
    /// Seek to a position in seconds.
    Seek(f64),
    /// Stop decoding and clean up.
    Stop,
    /// Set the loop flag.
    SetLoop(bool),
}

/// Handle to a running frame decoder.
pub struct FrameDecoder {
    cmd_tx: Sender<DecoderCommand>,
    frame_rx: Receiver<DecodedFrame>,
    _thread: thread::JoinHandle<()>,
}

impl std::fmt::Debug for FrameDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameDecoder")
            .field("has_frames", &self.frame_rx.try_recv().is_ok())
            .finish()
    }
}

impl FrameDecoder {
    /// Spawn a new decoder thread. Frames arrive on the returned receiver.
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            decoder_loop(cmd_rx, frame_tx);
        });

        Self { cmd_tx, frame_rx, _thread: thread }
    }

    /// Start decoding a video file, targeting the given cell dimensions.
    /// The output is scaled to `target_w × (target_h * 2)` pixels
    /// (doubled for half-block rendering).
    pub fn decode(&self, source: &str, target_w: u32, target_h: u32, fps: u32) {
        let _ = self.cmd_tx.send(DecoderCommand::Decode {
            source: source.to_string(),
            target_w,
            target_h,
            fps,
        });
    }

    /// Seek to a position in seconds.
    pub fn seek(&self, secs: f64) {
        let _ = self.cmd_tx.send(DecoderCommand::Seek(secs));
    }

    /// Stop the decoder.
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(DecoderCommand::Stop);
    }

    /// Set whether the decoder should loop on EOF.
    pub fn set_loop(&self, looping: bool) {
        let _ = self.cmd_tx.send(DecoderCommand::SetLoop(looping));
    }

    /// Try to receive the latest decoded frame (non-blocking).
    pub fn try_recv_frame(&self) -> Option<DecodedFrame> {
        // Drain to get the most recent frame.
        let mut latest = None;
        while let Ok(frame) = self.frame_rx.try_recv() {
            latest = Some(frame);
        }
        latest
    }
}

/// Probe a media file for metadata using ffprobe.
pub fn probe_video(source: &str) -> Option<VideoInfo> {
    let output = Command::new("ffprobe")
        .arg("-v").arg("error")
        .arg("-select_streams").arg("v:0")
        .arg("-show_entries").arg("stream=width,height,r_frame_rate,duration")
        .arg("-of").arg("json")
        .arg(source)
        .output()
        .ok()?;

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let stream = json.get("streams")?.get(0)?;

    let width = stream.get("width")?.as_u64()? as u32;
    let height = stream.get("height")?.as_u64()? as u32;
    let duration_secs = stream
        .get("duration")
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let fps = stream
        .get("r_frame_rate")
        .and_then(|r| r.as_str())
        .and_then(|s| {
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() == 2 {
                let n: f64 = parts[0].parse().ok()?;
                let d: f64 = parts[1].parse().ok()?;
                if d > 0.0 { Some(n / d) } else { None }
            } else {
                s.parse().ok()
            }
        })
        .unwrap_or(30.0);

    Some(VideoInfo { width, height, fps, duration_secs })
}

/// Spawn or restart an ffmpeg child for decoding.
fn spawn_ffmpeg(
    child: &mut Option<Child>,
    stdout_buf: &mut Option<ChildStdout>,
    source: &str,
    target_w: u32,
    pixel_h: u32,
    fps: u32,
    seek_secs: f64,
) {
    // Kill previous.
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    *stdout_buf = None;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-re")
        .arg("-i").arg(source)
        .arg("-f").arg("rawvideo")
        .arg("-pix_fmt").arg("rgb24")
        .arg("-vf").arg(format!(
            "scale={}:{}:flags=lanczos,fps={}",
            target_w, pixel_h, fps
        ))
        .arg("-").arg("-loglevel").arg("error")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if seek_secs > 0.0 {
        cmd.arg("-ss").arg(format!("{seek_secs:.2}"));
    }
    match cmd.spawn() {
        Ok(mut c) => {
            *stdout_buf = c.stdout.take();
            *child = Some(c);
        }
        Err(e) => {
            log::error!("Failed to spawn ffmpeg: {e}");
        }
    }
}

/// The decoder thread main loop.
fn decoder_loop(cmd_rx: Receiver<DecoderCommand>, frame_tx: Sender<DecodedFrame>) {
    let mut child: Option<Child> = None;
    let mut stdout_buf: Option<ChildStdout> = None;
    let mut pixel_buf: Vec<u8> = Vec::new();
    let mut frame_size = 0usize;
    let mut frame_w = 0usize;
    let mut frame_h = 0usize;
    let mut pts = 0.0f64;
    let mut fps = 30.0f64;
    // Stored state for looping and seeking.
    let mut source = String::new();
    let mut target_w: u32 = 0;
    let mut target_h: u32 = 0;
    let mut looping = false;

    loop {
        // Check for commands (non-blocking).
        match cmd_rx.try_recv() {
            Ok(DecoderCommand::Decode { source: src, target_w: tw, target_h: th, fps: tfps }) => {
                source = src.clone();
                target_w = tw;
                target_h = th;
                fps = tfps as f64;
                let pixel_h = th * 2;
                frame_w = tw as usize;
                frame_h = pixel_h as usize;
                frame_size = (tw * pixel_h * 3) as usize;
                pixel_buf.resize(frame_size, 0);
                pts = 0.0;
                spawn_ffmpeg(&mut child, &mut stdout_buf, &src, tw, pixel_h, tfps, 0.0);
            }
            Ok(DecoderCommand::Seek(secs)) => {
                pts = secs;
                if !source.is_empty() {
                    spawn_ffmpeg(&mut child, &mut stdout_buf, &source, target_w, target_h * 2, fps as u32, secs);
                }
            }
            Ok(DecoderCommand::SetLoop(l)) => {
                looping = l;
            }
            Ok(DecoderCommand::Stop) | Err(_) => {
                break;
            }
        }

        // Try to read a frame from ffmpeg stdout.
        if let Some(ref mut stdout) = stdout_buf {
            if frame_size > 0 {
                let mut bytes_read = 0;
                while bytes_read < frame_size {
                    match stdout.read(&mut pixel_buf[bytes_read..frame_size]) {
                        Ok(0) => {
                            // EOF — video finished.
                            stdout_buf = None;
                            if let Some(mut c) = child.take() {
                                let _ = c.kill();
                                let _ = c.wait();
                            }
                            // Loop: restart ffmpeg from the beginning.
                            if looping && !source.is_empty() {
                                pts = 0.0;
                                spawn_ffmpeg(&mut child, &mut stdout_buf, &source, target_w, target_h * 2, fps as u32, 0.0);
                            }
                            break;
                        }
                        Ok(n) => bytes_read += n,
                        Err(_) => {
                            stdout_buf = None;
                            break;
                        }
                    }
                }

                if bytes_read == frame_size {
                    let frame = DecodedFrame {
                        rgb: pixel_buf.clone(),
                        width: frame_w,
                        height: frame_h,
                        pts_secs: pts,
                    };
                    pts += 1.0 / fps;

                    // Send frame (drop if receiver is full — prevents backpressure stall).
                    let _ = frame_tx.send(frame);

                    // Pace frame delivery to match native FPS.
                    // ffmpeg -re already paces output, but this guard prevents
                    // bursts when the pipe buffer has pre-buffered data.
                    let frame_dur = std::time::Duration::from_secs_f64(1.0 / fps);
                    thread::sleep(frame_dur);
                }
            }
        } else {
            // No active decoder — sleep to avoid busy-wait.
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    // Cleanup.
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_video_nonexistent() {
        assert!(probe_video("/nonexistent/video.mp4").is_none());
    }
}
