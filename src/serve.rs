//! Loopback-only ASCII stream for OBS. Pixel display size is independent of the render grid.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub type Cells = Vec<Vec<(char, (u8, u8, u8))>>;

struct Frame {
    cells: Cells,
    w: usize,
    h: usize,
    seq: u64,
    json: String,
}

pub struct FrameStore {
    state: Mutex<Frame>,
    changed: Condvar,
    bg: (u8, u8, u8),
}

impl FrameStore {
    pub fn new(bg: (u8, u8, u8)) -> Self {
        Self {
            state: Mutex::new(Frame { cells: Vec::new(), w: 0, h: 0, seq: 0, json: String::new() }),
            changed: Condvar::new(),
            bg,
        }
    }

    /// Update dimensions and cells together. Pre-encode once; slow clients skip to the newest frame.
    pub fn set_frame(&self, cells: Cells) {
        let h = cells.len();
        let w = cells.iter().map(|row| row.len()).max().unwrap_or(0);
        let mut state = self.state.lock().unwrap();
        let seq = state.seq.wrapping_add(1);
        let json = encode_frame(seq, w, h, self.bg, &cells);
        *state = Frame { cells, w, h, seq, json };
        self.changed.notify_all();
    }
}

/// Exact RGB palette; a cell carries linear grid index, Unicode scalar, palette index.
/// Empty spaces are omitted, NOT shifted: their coordinates remain in the grid.
fn encode_frame(seq: u64, w: usize, h: usize, bg: (u8, u8, u8), cells: &Cells) -> String {
    let mut palette = Vec::new();
    let mut indices = HashMap::new();
    let mut entries = String::new();
    for (y, row) in cells.iter().enumerate() {
        for (x, &(glyph, rgb)) in row.iter().enumerate() {
            if glyph == ' ' { continue; }
            let color = *indices.entry(rgb).or_insert_with(|| {
                let index = palette.len();
                palette.push(rgb);
                index
            });
            if !entries.is_empty() { entries.push(','); }
            entries.push_str(&format!("[{},{},{}]", y * w + x, glyph as u32, color));
        }
    }
    let palette = palette.iter().map(|&(r, g, b)| format!("[{},{},{}]", r, g, b))
        .collect::<Vec<_>>().join(",");
    let produced_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis();
    format!("{{\"seq\":{},\"w\":{},\"h\":{},\"producedAt\":{},\"bg\":[{},{},{}],\"palette\":[{}],\"cells\":[{}]}}",
            seq, w, h, produced_at, bg.0, bg.1, bg.2, palette, entries)
}

fn escape(ch: char) -> String {
    match ch {
        '&' => "&amp;".into(), '<' => "&lt;".into(), '>' => "&gt;".into(),
        '"' => "&quot;".into(), '\'' => "&#39;".into(),
        c if c.is_control() => " ".into(), c => c.to_string(),
    }
}

/// Compatibility/debug endpoint. Keep leading AND trailing spaces to preserve geometry.
fn render_html(store: &FrameStore) -> String {
    let state = store.state.lock().unwrap();
    let mut out = String::new();
    for row in &state.cells {
        let mut previous = None;
        for &(ch, rgb) in row {
            if previous != Some(rgb) {
                if previous.is_some() { out.push_str("</span>"); }
                out.push_str(&format!("<span style=\"color:#{:02x}{:02x}{:02x}\">", rgb.0, rgb.1, rgb.2));
                previous = Some(rgb);
            }
            out.push_str(&escape(ch));
        }
        if previous.is_some() { out.push_str("</span>"); }
        out.push('\n');
    }
    out
}

fn page() -> &'static str {
    r#"<!doctype html><html><head><meta charset="utf-8"><title>Renderer · ASCII stream</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>html,body{margin:0;width:100%;height:100%;overflow:hidden;background:#000}
canvas{display:block;width:100%;height:100%}#status{position:fixed;left:8px;top:8px;color:#aaa;font:12px monospace;pointer-events:none}</style>
</head><body><canvas id="screen"></canvas><div id="status">等待 renderer 推帧…</div><script src="/viewer.js"></script></body></html>"#
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &str) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
           status, kind, body.len())?;
    stream.write_all(body.as_bytes())
}

fn handle_sse(mut stream: TcpStream, store: &FrameStore) -> std::io::Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n")?;
    let mut last = 0;
    loop {
        let mut state = store.state.lock().unwrap();
        if state.seq == last {
            let (next, timeout) = store.changed.wait_timeout(state, Duration::from_secs(5)).unwrap();
            state = next;
            if timeout.timed_out() && state.seq == last {
                drop(state);
                stream.write_all(b": keepalive\n\n")?;
                continue;
            }
        }
        if state.seq == last { continue; }
        last = state.seq;
        let body = state.json.clone();
        drop(state); // Never hold the frame mutex during socket writes.
        write!(stream, "id: {}\ndata: {}\n\n", last, body)?;
    }
}

fn handle(mut stream: TcpStream, store: Arc<FrameStore>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_nodelay(true)?;
    let mut request = Vec::new();
    let mut buf = [0; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf)?;
        if n == 0 { return Ok(()); }
        request.extend_from_slice(&buf[..n]);
        if request.len() > 8192 {
            return respond(&mut stream, "431 Request Header Fields Too Large", "text/plain", "header too large");
        }
    }
    let request = String::from_utf8_lossy(&request);
    let mut tokens = request.split_whitespace();
    if tokens.next() != Some("GET") {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain", "GET only");
    }
    let path = tokens.next().unwrap_or("/").split('?').next().unwrap_or("/");
    match path {
        "/stream" => handle_sse(stream, &store),
        "/frame" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", &render_html(&store)),
        "/snapshot" => {
            let state = store.state.lock().unwrap();
            let json = state.json.clone();
            drop(state);
            respond(&mut stream, "200 OK", "application/json", &json)
        }
        "/size" => {
            let state = store.state.lock().unwrap();
            let size = format!("{}x{}", state.w, state.h);
            drop(state);
            respond(&mut stream, "200 OK", "text/plain", &size)
        }
        "/viewer.js" => respond(&mut stream, "200 OK", "text/javascript; charset=utf-8", include_str!("viewer.js")),
        "/" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", page()),
        _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
    }
}

pub fn run(port: u16, store: Arc<FrameStore>) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => { log::error!("推流服务启动失败（127.0.0.1:{}）：{}", port, e); return; }
    };
    log::info!("ASCII 推流: http://127.0.0.1:{} （OBS 浏览器源；默认 385×350 小画面）", port);
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let store = store.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(stream, store) { log::debug!("推流客户端断开: {}", e); }
                });
            }
            Err(e) => log::warn!("推流接收连接失败: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgb_is_not_quantized_or_overflowed() {
        let json = encode_frame(1, 2, 1, (0,0,0), &vec![vec![(' ',(0,0,0)),('@',(255,254,128))]]);
        assert!(json.contains("\"palette\":[[255,254,128]]"));
        assert!(json.contains("\"cells\":[[1,64,0]]"));
    }
    #[test]
    fn numeric_glyphs_are_json_safe() {
        let cells = vec![vec![('"',(1,2,3)),('\\',(1,2,3)),('😺',(1,2,3))]];
        let json = encode_frame(1, 3, 1, (0,0,0), &cells);
        assert!(json.contains("[0,34,0],[1,92,0],[2,128570,0]"));
    }
    #[test]
    fn html_preserves_leading_spaces_and_escapes_content() {
        let store = FrameStore::new((0,0,0));
        store.set_frame(vec![vec![(' ',(0,0,0)),(' ',(0,0,0)),('<',(255,254,128)),('&',(255,254,128))]]);
        let html = render_html(&store);
        assert!(html.starts_with("<span style=\"color:#000000\">  </span>"));
        assert!(html.contains("&lt;&amp;"));
    }
    #[test]
    fn frame_dimensions_and_sequence_update_together() {
        let store = FrameStore::new((0,0,0));
        store.set_frame(vec![vec![('@',(1,2,3)); 128]; 64]);
        store.set_frame(vec![vec![('@',(1,2,3)); 40]; 20]);
        let state=store.state.lock().unwrap();
        assert_eq!((state.w,state.h,state.seq), (40,20,2));
        assert!(state.json.contains("\"w\":40,\"h\":20"));
    }
}
