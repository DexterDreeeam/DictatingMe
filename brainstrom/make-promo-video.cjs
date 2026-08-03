// 把 promo-footage.html 录成视频（画面 + 声音）。
//
// 为什么这么绕：
//   - ffmpeg 在这台机器上看不到任何 loopback 音频设备，录不了系统声音；
//   - 页面的音效和 BGM 是 WebAudio 实时合成的，配音是 <audio> 播的。
//   所以音频只能在浏览器内部录：页面把所有声音汇到一条总线，录制模式下
//   接一个 MediaStreamDestination + MediaRecorder，录完用 base64 递出来。
//
//   画面走 CDP 的 Page.screencast，每帧自带时间戳，用 ffmpeg concat 按真实
//   间隔还原，避免固定帧率带来的音画漂移。
//
// 用法：node make-promo-video.js [landscape|portrait|both]

const { spawn, spawnSync } = require('node:child_process');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');

const REPO = path.resolve(__dirname, '..');
const OUT_DIR = path.join(REPO, 'brainstrom', 'promo-video');
const PORT_HTTP = 8123;
const PORT_CDP = 9334;

const CHROME = [
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
].find(p => fs.existsSync(p));

const PRESETS = {
  landscape: { w: 1920, h: 1080, name: 'DictatingMe-1080p.mp4', query: '' },
  portrait:  { w: 1080, h: 1920, name: 'DictatingMe-vertical.mp4', query: '&vertical=1' },
};

const sleep = ms => new Promise(r => setTimeout(r, ms));

// ---------------------------------------------------------------- 静态服务器
// 必须走 http 而不是 file://：createMediaElementSource 对 file:// 的媒体会
// 因为跨源污染录出静音。
function serve(root, port) {
  const types = { '.html': 'text/html; charset=utf-8', '.mp3': 'audio/mpeg', '.png': 'image/png', '.txt': 'text/plain; charset=utf-8' };
  const server = http.createServer((req, res) => {
    const rel = decodeURIComponent(req.url.split('?')[0]).replace(/^\/+/, '');
    const file = path.join(root, rel);
    if (!file.startsWith(root) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404); return res.end('not found');
    }
    res.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise(r => server.listen(port, '127.0.0.1', () => r(server)));
}

// ---------------------------------------------------------------- CDP 小客户端
class Cdp {
  constructor(ws) { this.ws = ws; this.n = 0; this.pending = new Map(); this.handlers = []; }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => {
      ws.addEventListener('open', res, { once: true });
      ws.addEventListener('error', rej, { once: true });
    });
    const c = new Cdp(ws);
    ws.addEventListener('message', ev => {
      const m = JSON.parse(ev.data);
      if (m.id && c.pending.has(m.id)) {
        const { resolve, reject } = c.pending.get(m.id);
        c.pending.delete(m.id);
        m.error ? reject(new Error(m.method + ': ' + JSON.stringify(m.error))) : resolve(m.result);
      } else if (m.method) {
        c.handlers.forEach(h => h(m));
      }
    });
    return c;
  }
  send(method, params = {}, sessionId) {
    const id = ++this.n;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }
  on(fn) { this.handlers.push(fn); }
  close() { this.ws.close(); }
}

// ---------------------------------------------------------------- 录一条
async function record(preset, label) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'promo-rec-'));
  const framesDir = path.join(work, 'frames');
  fs.mkdirSync(framesDir);

  const chrome = spawn(CHROME, [
    '--headless=new',
    `--remote-debugging-port=${PORT_CDP}`,
    `--window-size=${preset.w},${preset.h}`,
    '--autoplay-policy=no-user-gesture-required',
    '--disable-gpu',
    '--hide-scrollbars',
    '--force-device-scale-factor=1',
    '--no-first-run',
    '--mute-audio=false',
    `--user-data-dir=${path.join(work, 'profile')}`,
    'about:blank',
  ], { stdio: 'ignore' });

  let cdp;
  try {
    let version;
    for (let i = 0; i < 60; i++) {
      try { version = await (await fetch(`http://127.0.0.1:${PORT_CDP}/json/version`)).json(); break; }
      catch { await sleep(250); }
    }
    if (!version) throw new Error('DevTools 端口没起来');

    cdp = await Cdp.connect(version.webSocketDebuggerUrl);
    const url = `http://127.0.0.1:${PORT_HTTP}/brainstrom/promo-footage.html?record=1${preset.query}`;
    const { targetId } = await cdp.send('Target.createTarget', { url });
    const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });

    await cdp.send('Page.enable', {}, sessionId);
    await cdp.send('Runtime.enable', {}, sessionId);
    // 视口必须显式设，否则 screencast 尺寸跟着窗口而不是我们要的画布
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: preset.w, height: preset.h, deviceScaleFactor: 1, mobile: false,
    }, sessionId);

    // 等页面里的 __rec 就绪
    for (let i = 0; i < 60; i++) {
      const r = await cdp.send('Runtime.evaluate', { expression: 'typeof window.__rec', returnByValue: true }, sessionId);
      if (r.result.value === 'object') break;
      await sleep(250);
    }

    const armed = await cdp.send('Runtime.evaluate', {
      expression: 'window.__rec.arm()', awaitPromise: true, returnByValue: true,
    }, sessionId);
    console.log(`  音频就绪: ${JSON.stringify(armed.result.value)}`);

    // ---- 抓帧 ----
    const frames = [];
    cdp.on(m => {
      if (m.method !== 'Page.screencastFrame') return;
      const p = m.params;
      const file = path.join(framesDir, `f${String(frames.length).padStart(5, '0')}.jpg`);
      fs.writeFileSync(file, Buffer.from(p.data, 'base64'));
      frames.push({ file, t: p.metadata.timestamp });
      cdp.send('Page.screencastFrameAck', { sessionId: p.sessionId }, sessionId).catch(() => {});
    });

    await cdp.send('Page.startScreencast', {
      format: 'jpeg', quality: 92, maxWidth: preset.w, maxHeight: preset.h, everyNthFrame: 1,
    }, sessionId);

    const run = await cdp.send('Runtime.evaluate', {
      expression: 'window.__rec.run()', awaitPromise: true, returnByValue: true, timeout: 180000,
    }, sessionId);

    await sleep(400);                       // 收尾帧
    await cdp.send('Page.stopScreencast', {}, sessionId);

    const { durationMs, audioB64, bytes } = run.result.value;
    console.log(`  画面 ${frames.length} 帧 / 音频 ${(bytes / 1024).toFixed(0)} KB / 时长 ${(durationMs / 1000).toFixed(1)}s`);
    if (!frames.length) throw new Error('一帧都没抓到');
    if (!bytes) throw new Error('音轨是空的');

    const audioFile = path.join(work, 'audio.webm');
    fs.writeFileSync(audioFile, Buffer.from(audioB64, 'base64'));

    // ---- 合成 ----
    // 用每帧的真实时间戳，而不是假设固定帧率，音画才不会越走越偏
    const list = frames.map((f, i) => {
      const next = frames[i + 1];
      const dur = next ? (next.t - f.t) : (durationMs / 1000 - (f.t - frames[0].t));
      return `file '${f.file.replace(/\\/g, '/')}'\nduration ${Math.max(dur, 1 / 120).toFixed(6)}`;
    }).join('\n');
    const listFile = path.join(work, 'frames.txt');
    fs.writeFileSync(listFile, list + `\nfile '${frames[frames.length - 1].file.replace(/\\/g, '/')}'\n`);

    fs.mkdirSync(OUT_DIR, { recursive: true });
    const outFile = path.join(OUT_DIR, preset.name);
    const args = [
      '-y', '-hide_banner', '-loglevel', 'error',
      '-f', 'concat', '-safe', '0', '-i', listFile,
      '-i', audioFile,
      '-vf', `fps=30,scale=${preset.w}:${preset.h}:flags=lanczos,format=yuv420p`,
      '-c:v', 'libx264', '-preset', 'slow', '-crf', '18',
      '-c:a', 'aac', '-b:a', '192k',
      '-movflags', '+faststart',
      '-shortest', outFile,
    ];
    const ff = spawnSync('ffmpeg', args, { encoding: 'utf8' });
    if (ff.status !== 0) throw new Error('ffmpeg 失败:\n' + (ff.stderr || ff.stdout));

    const size = fs.statSync(outFile).size;
    console.log(`  -> ${outFile}  ${(size / 1024 / 1024).toFixed(1)} MB`);
    return outFile;
  } finally {
    if (cdp) cdp.close();
    chrome.kill();
    await sleep(600);
    fs.rmSync(work, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------- main
(async () => {
  if (!CHROME) throw new Error('找不到 Chrome 或 Edge');
  const which = (process.argv[2] || 'both').toLowerCase();
  const jobs = which === 'both' ? ['landscape', 'portrait'] : [which];
  for (const j of jobs) if (!PRESETS[j]) throw new Error(`未知目标 '${j}'，可选 landscape / portrait / both`);

  // 服务整个仓库而不是只服务 brainstrom：页面用 ../runtime/icons 和
  // ../ui/assets 引用 logo，root 卡在 brainstrom 的话这些请求会被拦成 404，
  // 录出来 logo 是破图。
  const server = await serve(REPO, PORT_HTTP);
  try {
    for (const j of jobs) {
      const p = PRESETS[j];
      console.log(`\n[${j}] ${p.w}x${p.h}`);
      await record(p, j);
    }
  } finally {
    server.close();
  }
  console.log(`\n完成，输出在 ${OUT_DIR}`);
})().catch(e => { console.error('\n失败:', e.message); process.exit(1); });
