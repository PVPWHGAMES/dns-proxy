const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

// 生成 PNG 数据
function createPNG(width, height) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  // IHDR
  const ihdrData = Buffer.alloc(13);
  ihdrData.writeUInt32BE(width, 0);
  ihdrData.writeUInt32BE(height, 4);
  ihdrData.writeUInt8(8, 8);   // bit depth
  ihdrData.writeUInt8(6, 9);   // RGBA (带透明通道)
  ihdrData.writeUInt8(0, 10);
  ihdrData.writeUInt8(0, 11);
  ihdrData.writeUInt8(0, 12);
  const ihdr = createChunk('IHDR', ihdrData);

  // IDAT - 蓝色圆形图标
  const raw = [];
  const cx = width / 2, cy = height / 2, r = width * 0.42;
  for (let y = 0; y < height; y++) {
    raw.push(0); // filter
    for (let x = 0; x < width; x++) {
      const dx = x - cx, dy = y - cy;
      const dist = Math.sqrt(dx * dx + dy * dy);
      if (dist <= r) {
        const t = dist / r;
        raw.push(
          Math.floor(37 + t * 30),   // R
          Math.floor(99 + t * 50),   // G
          Math.floor(235 - t * 20),  // B
          255                         // A
        );
      } else if (dist <= r + 1.5) {
        // 抗锯齿边缘
        const alpha = Math.max(0, Math.floor(255 * (r + 1.5 - dist) / 1.5));
        raw.push(59, 130, 246, alpha);
      } else {
        raw.push(0, 0, 0, 0); // 透明
      }
    }
  }
  const compressed = zlib.deflateSync(Buffer.from(raw));
  const idat = createChunk('IDAT', compressed);
  const iend = createChunk('IEND', Buffer.alloc(0));

  return Buffer.concat([signature, ihdr, idat, iend]);
}

function createChunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeB = Buffer.from(type, 'ascii');
  const crcVal = crc32(Buffer.concat([typeB, data]));
  const crcB = Buffer.alloc(4);
  crcB.writeUInt32BE(crcVal >>> 0, 0);
  return Buffer.concat([len, typeB, data, crcB]);
}

function crc32(buf) {
  let c = 0xFFFFFFFF;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let j = 0; j < 8; j++) c = (c >>> 1) ^ (c & 1 ? 0xEDB88320 : 0);
  }
  return (c ^ 0xFFFFFFFF) >>> 0;
}

// 生成正确的 ICO 文件
function createICO(pngEntries) {
  // ICO Header: reserved(2) + type(2) + count(2)
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);     // reserved
  header.writeUInt16LE(1, 2);     // type: 1 = ICO
  header.writeUInt16LE(pngEntries.length, 4);

  // Directory entries + image data
  const entries = [];
  const images = [];
  let offset = 6 + pngEntries.length * 16;

  for (const png of pngEntries) {
    // Directory entry: 16 bytes
    const entry = Buffer.alloc(16);
    entry.writeUInt8(png.width >= 256 ? 0 : png.width, 0);
    entry.writeUInt8(png.height >= 256 ? 0 : png.height, 1);
    entry.writeUInt8(0, 2);    // color palette
    entry.writeUInt8(0, 3);    // reserved
    entry.writeUInt16LE(1, 4); // color planes
    entry.writeUInt16LE(32, 6);// bits per pixel
    entry.writeUInt32LE(png.data.length, 8);
    entry.writeUInt32LE(offset, 12);
    entries.push(entry);
    images.push(png.data);
    offset += png.data.length;
  }

  return Buffer.concat([header, ...entries, ...images]);
}

const iconsDir = path.join(__dirname, '..', 'src-tauri', 'icons');

// 生成 PNG 图标
const pngSizes = [
  { name: '32x32.png', size: 32 },
  { name: '128x128.png', size: 128 },
  { name: '128x128@2x.png', size: 256 },
];

for (const { name, size } of pngSizes) {
  const png = createPNG(size, size);
  fs.writeFileSync(path.join(iconsDir, name), png);
  console.log(`Created ${name} (${size}x${size})`);
}

// 生成 ICO 文件（包含 16, 32, 48, 256 尺寸）
const icoSizes = [16, 32, 48, 256];
const icoEntries = icoSizes.map(size => ({
  width: size,
  height: size,
  data: createPNG(size, size)
}));
const ico = createICO(icoEntries);
fs.writeFileSync(path.join(iconsDir, 'icon.ico'), ico);
console.log('Created icon.ico (multi-size: 16,32,48,256)');

// ICNS 用 PNG 替代（macOS 用，Windows 不校验）
const icns = createPNG(512, 512);
fs.writeFileSync(path.join(iconsDir, 'icon.icns'), icns);
console.log('Created icon.icns');

console.log('\nDone! All icons generated.');
