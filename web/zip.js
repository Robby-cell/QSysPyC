// Pure-JS zero-dependency ZIP archive creator and reader for QSysPy Studio.
// Supports creating standard STORED (uncompressed) ZIP archives and reading
// both STORED (method 0) and DEFLATE (method 8, via DecompressionStream) ZIP files.

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[i] = c >>> 0;
  }
  return table;
})();

function crc32(bytes) {
  let crc = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) {
    crc = CRC_TABLE[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function dateToDos(date = new Date()) {
  const year = Math.max(1980, date.getFullYear());
  const dosTime =
    ((date.getHours() & 0x1f) << 11) |
    ((date.getMinutes() & 0x3f) << 5) |
    ((date.getSeconds() >> 1) & 0x1f);
  const dosDate =
    (((year - 1980) & 0x7f) << 9) | (((date.getMonth() + 1) & 0x0f) << 5) | (date.getDate() & 0x1f);
  return { dosTime, dosDate };
}

/**
 * Create a standard `.zip` Blob from an array of `{ path, content }` entries.
 * @param {Array<{ path: string, content: string | Uint8Array }>} entries
 * @returns {Blob}
 */
export function createZipBlob(entries) {
  const encoder = new TextEncoder();
  const localParts = [];
  const centralParts = [];
  let offset = 0;
  const { dosTime, dosDate } = dateToDos(new Date());

  for (const entry of entries) {
    const cleanPath = entry.path.replace(/^\/+/, "").replace(/\\/g, "/");
    const nameBytes = encoder.encode(cleanPath);
    const dataBytes =
      typeof entry.content === "string" ? encoder.encode(entry.content) : entry.content;
    const crc = crc32(dataBytes);
    const size = dataBytes.byteLength;

    // Local file header (30 bytes + filename)
    const localHeader = new Uint8Array(30 + nameBytes.byteLength);
    const lv = new DataView(localHeader.buffer);
    lv.setUint32(0, 0x04034b50, true); // Local file header signature
    lv.setUint16(4, 20, true); // Version needed to extract (2.0)
    lv.setUint16(6, 0x0800, true); // General purpose bit flag (UTF-8 filename)
    lv.setUint16(8, 0, true); // Compression method: 0 (STORED)
    lv.setUint16(10, dosTime, true);
    lv.setUint16(12, dosDate, true);
    lv.setUint32(14, crc, true);
    lv.setUint32(18, size, true);
    lv.setUint32(22, size, true);
    lv.setUint16(26, nameBytes.byteLength, true);
    lv.setUint16(28, 0, true); // Extra field length
    localHeader.set(nameBytes, 30);

    localParts.push(localHeader, dataBytes);

    // Central directory file header (46 bytes + filename)
    const centralHeader = new Uint8Array(46 + nameBytes.byteLength);
    const cv = new DataView(centralHeader.buffer);
    cv.setUint32(0, 0x02014b50, true); // Central directory signature
    cv.setUint16(4, 20, true); // Version made by
    cv.setUint16(6, 20, true); // Version needed
    cv.setUint16(8, 0x0800, true); // UTF-8 flag
    cv.setUint16(10, 0, true); // Compression method: 0 (STORED)
    cv.setUint16(12, dosTime, true);
    cv.setUint16(14, dosDate, true);
    cv.setUint32(16, crc, true);
    cv.setUint32(20, size, true);
    cv.setUint32(24, size, true);
    cv.setUint16(28, nameBytes.byteLength, true);
    cv.setUint16(30, 0, true); // Extra length
    cv.setUint16(32, 0, true); // Comment length
    cv.setUint16(34, 0, true); // Disk number
    cv.setUint16(36, 0, true); // Internal attrs
    cv.setUint32(38, 0, true); // External attrs
    cv.setUint32(42, offset, true); // Relative offset of local header
    centralHeader.set(nameBytes, 46);

    centralParts.push(centralHeader);
    offset += localHeader.byteLength + size;
  }

  const centralSize = centralParts.reduce((acc, part) => acc + part.byteLength, 0);

  // End of central directory record (22 bytes)
  const eocd = new Uint8Array(22);
  const ev = new DataView(eocd.buffer);
  ev.setUint32(0, 0x06054b50, true); // EOCD signature
  ev.setUint16(4, 0, true); // Disk number
  ev.setUint16(6, 0, true); // Disk with central dir
  ev.setUint16(8, entries.length, true); // Entries on this disk
  ev.setUint16(10, entries.length, true); // Total entries
  ev.setUint32(12, centralSize, true); // Central directory byte size
  ev.setUint32(16, offset, true); // Central directory offset
  ev.setUint16(20, 0, true); // Comment length

  return new Blob([...localParts, ...centralParts, eocd], { type: "application/zip" });
}

/**
 * Extract text files from a `.zip` ArrayBuffer.
 * Supports STORED (0) and DEFLATE (8) entries.
 * @param {ArrayBuffer} buffer
 * @returns {Promise<Array<{ path: string, content: string }>>}
 */
export async function extractZipEntries(buffer) {
  const bytes = new Uint8Array(buffer);
  const view = new DataView(buffer);
  const decoder = new TextDecoder("utf-8");

  // Locate End of Central Directory (EOCD) scanning backwards from the end
  let eocdOffset = -1;
  const minEocdSearch = Math.max(0, bytes.byteLength - 65557);
  for (let i = bytes.byteLength - 22; i >= minEocdSearch; i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      eocdOffset = i;
      break;
    }
  }
  if (eocdOffset === -1) {
    throw new Error("Invalid ZIP archive: End of Central Directory record not found.");
  }

  const totalEntries = view.getUint16(eocdOffset + 10, true);
  let cdOffset = view.getUint32(eocdOffset + 16, true);
  const results = [];

  for (let i = 0; i < totalEntries; i++) {
    if (cdOffset + 46 > bytes.byteLength || view.getUint32(cdOffset, true) !== 0x02014b50) {
      break;
    }
    const method = view.getUint16(cdOffset + 10, true);
    const compressedSize = view.getUint32(cdOffset + 20, true);
    const nameLen = view.getUint16(cdOffset + 28, true);
    const extraLen = view.getUint16(cdOffset + 30, true);
    const commentLen = view.getUint16(cdOffset + 32, true);
    const localHeaderOffset = view.getUint32(cdOffset + 42, true);

    const rawName = decoder.decode(bytes.subarray(cdOffset + 46, cdOffset + 46 + nameLen));
    cdOffset += 46 + nameLen + extraLen + commentLen;

    // Skip directories and hidden OS metadata
    if (rawName.endsWith("/") || rawName.includes("__MACOSX/") || rawName.endsWith(".DS_Store")) {
      continue;
    }

    const localNameLen = view.getUint16(localHeaderOffset + 26, true);
    const localExtraLen = view.getUint16(localHeaderOffset + 28, true);
    const dataStart = localHeaderOffset + 30 + localNameLen + localExtraLen;
    const compressedBytes = bytes.subarray(dataStart, dataStart + compressedSize);

    let fileBytes;
    if (method === 0) {
      fileBytes = compressedBytes;
    } else if (method === 8 && typeof DecompressionStream !== "undefined") {
      const ds = new DecompressionStream("deflate-raw");
      const writer = ds.writable.getWriter();
      writer.write(compressedBytes);
      writer.close();
      const resp = new Response(ds.readable);
      const ab = await resp.arrayBuffer();
      fileBytes = new Uint8Array(ab);
    } else {
      continue;
    }

    results.push({
      path: rawName.replace(/^\/+/, ""),
      content: decoder.decode(fileBytes),
    });
  }

  // Strip common single top-level directory prefix if all files share one
  if (results.length > 0) {
    const firstSlash = results[0].path.indexOf("/");
    if (firstSlash > 0) {
      const prefix = results[0].path.slice(0, firstSlash + 1);
      if (results.every((r) => r.path.startsWith(prefix))) {
        for (const r of results) {
          r.path = r.path.slice(prefix.length);
        }
      }
    }
  }

  return results.filter((r) => r.path.length > 0);
}
