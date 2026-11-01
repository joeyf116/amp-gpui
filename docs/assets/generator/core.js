// The tone generator's logic, shared by the docs page and the Claude artifact.
// Nothing here touches the page or the network, so every route gets the same
// prompt, the same checks, and the same download.
(() => {
  const catalog = globalThis.AmpToneCatalog;

  // NAM amp and pedal blocks do nothing without a file, so a generated tone
  // sticks to blocks that play on a fresh install.
  const KINDS = ["gate", "compressor", "drive", "tube", "cab", "chorus", "delay", "reverb"];

  const BLOCK_NOTES = {
    gate: "Noise gate. Use on medium and high gain. Threshold about -70 to -55.",
    compressor:
      "Feed-forward compressor with a 12 ms attack, for cleans, funk, and sustain. Sits before the amp.",
    drive:
      "Asymmetric soft clipper, like an overdrive pedal. Low drive with level near 0 pushes the amp; high drive is the dirt itself.",
    tube:
      "The Understory amp, an original tube amp model and the heart of the tone. Gain runs up to 46 dB and trims its own output, so more gain adds dirt, not volume. Bass, mid, and treble are flat at 5, at 2.4 dB per step, centered on 140 Hz, 700 Hz, and 2.6 kHz. Presence is a shelf at 4.8 kHz. Master sets the level.",
    cab: "A built-in speaker cabinet voice with a bass bump, a 2.5 kHz peak, and a steep roll-off above 5 kHz. Always use one after the amp.",
    chorus: "One modulated delay line. Goes after the cab.",
    delay: "Mono delay. Goes after the cab. Time in ms, up to 1500.",
    reverb: "Algorithmic reverb with lows filtered out of the tail. Goes last.",
  };

  const params = (kind) => catalog.blocks[kind].params;

  function blockReference() {
    return KINDS.map((kind) => {
      const controls = params(kind)
        .map((p) => `${p.id} ${p.min} to ${p.max}${p.unit ? ` ${p.unit}` : ""} (default ${p.default})`)
        .join("; ");
      return `- ${kind}: ${BLOCK_NOTES[kind]} Params: ${controls}.`;
    }).join("\n");
  }

  function exampleTones() {
    return catalog.examples
      .map(({ name, tags, notes, blocks }) => JSON.stringify({ name, tags, notes, blocks }))
      .join("\n");
  }

  const OUTPUT_FORMAT = `{"name": string, "tags": [string], "notes": string, "blocks": [{"kind": string, "enabled": true, "params": {"<param id>": number}}]}`;

  function instructions() {
    return `You design guitar tones for Amp GPUI, a desktop amp simulator. Turn the player's description into one complete signal chain, built only from these blocks:

${blockReference()}

How to build the chain:
- Blocks run in array order. Use the usual order: gate, compressor, drive, tube, cab, chorus, delay, reverb. Leave out what the tone doesn't need, and set "enabled" to true on every block you include.
- Always include exactly one tube and one cab.
- Set every param of every block you include. Stay inside each range.
- If the player names a song or artist, use what you know about that sound, and aim for its most famous part unless they name another.

Levels matter, because nobody can audition this before it's saved:
- The amp's master and any drive level set how loud the tone is. Follow the examples: they all land within 5 dB of each other and never clip.
- Clean tones use gain under about 2.5 and master near 6. High gain tones use gain near 7 or 8 with master near 3 or 4, since gain adds a lot of level before the trim.
- A drive block in front of the amp adds level, so lower master by about 1 when you use one. A compressor's level adds to the total too.
- Keep delay and reverb mix well under 50 unless the player asks for an ambient wash. Keep delay feedback under 60.

Writing the preset:
- "name" is a short, original name of one to three words in the spirit of the examples, which are named after the forest, stone, and weather. Never use a trademark, band, artist, or song in the name, and never reuse an example's name.
- "tags" are one to three lowercase words, such as "clean", "crunch", "high gain", "lead", "ambient".
- "notes" is two sentences or fewer on how the tone plays. Plain words only. If the tone leans on a real amp or pedal, name it here. If the sound needs something these blocks can't do, such as wah, phaser, tremolo, pitch shifting, or stereo, say so plainly.

These are real presets from the app. Use them to calibrate levels and settings:

${exampleTones()}

Reply with only one JSON object in this shape, with no other text:

${OUTPUT_FORMAT}`;
  }

  // The player's words go in a tagged block, last, so the instructions above
  // stay the same from one request to the next.
  function request(description) {
    return `<tone_request>\n${description.trim()}\n</tone_request>`;
  }

  // Everything in one message, for routes with no separate system prompt.
  function fullPrompt(description) {
    return `${instructions()}\n\n${request(description)}`;
  }

  // Claude's structured output, which leaves ranges to normalize() below
  // because the API's schemas don't support numeric limits.
  function responseSchema() {
    return {
      type: "object",
      additionalProperties: false,
      required: ["name", "tags", "notes", "blocks"],
      properties: {
        name: { type: "string" },
        tags: { type: "array", items: { type: "string" } },
        notes: { type: "string" },
        blocks: {
          type: "array",
          items: {
            anyOf: KINDS.map((kind) => ({
              type: "object",
              additionalProperties: false,
              required: ["kind", "enabled", "params"],
              properties: {
                kind: { const: kind },
                enabled: { type: "boolean" },
                params: {
                  type: "object",
                  additionalProperties: false,
                  required: params(kind).map((p) => p.id),
                  properties: Object.fromEntries(
                    params(kind).map((p) => [p.id, { type: "number" }]),
                  ),
                },
              },
            })),
          },
        },
      },
    };
  }

  // Finds the preset in a reply that may have a code fence or a sentence around it.
  function parseReply(text) {
    const trimmed = text.trim();
    const fence = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/);
    const candidates = [trimmed, fence?.[1], trimmed.slice(trimmed.indexOf("{"), trimmed.lastIndexOf("}") + 1)];
    for (const candidate of candidates) {
      try {
        if (candidate) return JSON.parse(candidate);
      } catch {
        // Try the next shape.
      }
    }
    throw new Error("That reply doesn't contain a preset. Copy Claude's whole reply and try again.");
  }

  function slug(name) {
    let out = "";
    for (const character of name.toLowerCase()) {
      if (/[\p{L}\p{N}]/u.test(character)) out += character;
      else if (out && !out.endsWith("-")) out += "-";
    }
    return out.replace(/-+$/, "");
  }

  function clampParams(kind, raw) {
    return Object.fromEntries(
      params(kind).map((p) => {
        const value = Number(raw?.[p.id]);
        const safe = Number.isFinite(value) ? Math.min(p.max, Math.max(p.min, value)) : p.default;
        return [p.id, Math.round(safe * 100) / 100];
      }),
    );
  }

  // The checks the app would apply on load, run early so a bad reply gets a
  // clear message here instead of an odd tone later. Input is untrusted.
  function normalize(raw) {
    const source = Array.isArray(raw) ? raw[0] : raw;
    if (!source || typeof source !== "object" || !Array.isArray(source.blocks)) {
      throw new Error("That isn't an Amp GPUI preset: it has no list of blocks.");
    }
    const warnings = [];
    const blocks = [];
    for (const block of source.blocks) {
      if (!KINDS.includes(block?.kind)) {
        warnings.push(`Removed a block the generator doesn't support: ${String(block?.kind)}.`);
        continue;
      }
      blocks.push({
        kind: block.kind,
        enabled: block.enabled !== false,
        params: clampParams(block.kind, block.params),
      });
    }
    if (!blocks.some((block) => block.kind === "tube")) {
      throw new Error("That preset has no amp, so it wouldn't make a usable tone.");
    }
    if (!blocks.some((block) => block.kind === "cab")) {
      warnings.push("This tone has no cabinet, so it may sound harsh.");
    }
    const name = String(source.name ?? "").trim().slice(0, 60);
    return {
      preset: {
        schema: catalog.schema,
        name: slug(name) ? name : "Generated Tone",
        author: "Amp GPUI tone generator",
        tags: (Array.isArray(source.tags) ? source.tags : []).map(String).slice(0, 5),
        notes: String(source.notes ?? "").trim().slice(0, 600),
        blocks,
      },
      warnings,
    };
  }

  function presetJson(preset) {
    return `${JSON.stringify(preset, null, 2)}\n`;
  }

  // A one-line-per-block summary to show before anything is downloaded.
  function describeChain(preset) {
    return preset.blocks.map((block) => ({
      title: catalog.blocks[block.kind].title,
      settings: params(block.kind).map((p) => `${p.name} ${block.params[p.id]}${p.unit ? ` ${p.unit}` : ""}`),
    }));
  }

  const CRC_TABLE = Array.from({ length: 256 }, (_, index) => {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    return value >>> 0;
  });

  function crc32(bytes) {
    let crc = 0xffffffff;
    for (const byte of bytes) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
    return (crc ^ 0xffffffff) >>> 0;
  }

  // A zip with one stored file. It's small enough that compression gains
  // nothing, and writing it here keeps the page free of libraries.
  function zip(path, bytes) {
    const name = new TextEncoder().encode(path);
    const crc = crc32(bytes);
    const local = new DataView(new ArrayBuffer(30));
    const central = new DataView(new ArrayBuffer(46));
    const end = new DataView(new ArrayBuffer(22));

    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, 20, true);
    local.setUint16(6, 0x0800, true); // The name is UTF-8.
    local.setUint32(14, crc, true);
    local.setUint32(18, bytes.length, true);
    local.setUint32(22, bytes.length, true);
    local.setUint16(26, name.length, true);

    central.setUint32(0, 0x02014b50, true);
    central.setUint16(4, 20, true);
    central.setUint16(6, 20, true);
    central.setUint16(8, 0x0800, true);
    central.setUint32(16, crc, true);
    central.setUint32(20, bytes.length, true);
    central.setUint32(24, bytes.length, true);
    central.setUint16(28, name.length, true);

    const centralOffset = 30 + name.length + bytes.length;
    end.setUint32(0, 0x06054b50, true);
    end.setUint16(8, 1, true);
    end.setUint16(10, 1, true);
    end.setUint32(12, 46 + name.length, true);
    end.setUint32(16, centralOffset, true);

    const parts = [local.buffer, name, bytes, central.buffer, name, end.buffer];
    const out = new Uint8Array(parts.reduce((size, part) => size + part.byteLength, 0));
    let offset = 0;
    for (const part of parts) {
      out.set(new Uint8Array(part.buffer ?? part), offset);
      offset += part.byteLength;
    }
    return out;
  }

  // A tone pack is a small library, so the app's Import reads it as it is.
  function tonePack(preset) {
    const file = slug(preset.name);
    const bytes = new TextEncoder().encode(presetJson(preset));
    return { filename: `${file}-tone-pack.zip`, data: zip(`presets/${file}.json`, bytes) };
  }

  globalThis.AmpTones = {
    describeChain,
    fullPrompt,
    instructions,
    normalize,
    parseReply,
    presetJson,
    request,
    responseSchema,
    slug,
    tonePack,
  };
})();
