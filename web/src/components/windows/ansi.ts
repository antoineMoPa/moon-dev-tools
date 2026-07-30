import Anser from "anser";

/// Turns the raw stdout/stderr of an agent into styled spans. Anser parses the escape
/// sequences; the colours below are ours, picked to stay readable on the dark log surface.

export type AnsiStyle = {
  color?: string;
  backgroundColor?: string;
  fontWeight?: "bold";
  fontStyle?: "italic";
  textDecoration?: string;
  opacity?: number;
};

export type AnsiSpan = {
  text: string;
  style: AnsiStyle;
};

const mapAnsiColorClassToColor = {
  "ansi-black": "#5b6673",
  "ansi-red": "#ff7b72",
  "ansi-green": "#7ee787",
  "ansi-yellow": "#e3b341",
  "ansi-blue": "#79c0ff",
  "ansi-magenta": "#d2a8ff",
  "ansi-cyan": "#56d4dd",
  "ansi-white": "#edf3fb",
  "ansi-bright-black": "#8b949e",
  "ansi-bright-red": "#ffa198",
  "ansi-bright-green": "#a5f3ae",
  "ansi-bright-yellow": "#f2cc60",
  "ansi-bright-blue": "#a5d6ff",
  "ansi-bright-magenta": "#e2c5ff",
  "ansi-bright-cyan": "#88e2e8",
  "ansi-bright-white": "#ffffff",
} as Record<string, string>;

/// The first sixteen entries of the 256-colour palette, in palette order.
const ANSI_PALETTE_CLASS_ORDER = [
  "ansi-black",
  "ansi-red",
  "ansi-green",
  "ansi-yellow",
  "ansi-blue",
  "ansi-magenta",
  "ansi-cyan",
  "ansi-white",
  "ansi-bright-black",
  "ansi-bright-red",
  "ansi-bright-green",
  "ansi-bright-yellow",
  "ansi-bright-blue",
  "ansi-bright-magenta",
  "ansi-bright-cyan",
  "ansi-bright-white",
];

const mapDecorationToStyle = {
  bold: { fontWeight: "bold" },
  dim: { opacity: 0.72 },
  italic: { fontStyle: "italic" },
  underline: { textDecoration: "underline" },
  strikethrough: { textDecoration: "line-through" },
} as Record<string, AnsiStyle>;

const ANSI_CUBE_STEPS = [0, 95, 135, 175, 215, 255];

/// The 6×6×6 colour cube and the grey ramp above it.
function paletteColor(paletteIndex: number): string {
  if (paletteIndex < 16) {
    return mapAnsiColorClassToColor[ANSI_PALETTE_CLASS_ORDER[paletteIndex]];
  }
  if (paletteIndex < 232) {
    const offset = paletteIndex - 16;
    const red = ANSI_CUBE_STEPS[Math.floor(offset / 36)];
    const green = ANSI_CUBE_STEPS[Math.floor(offset / 6) % 6];
    const blue = ANSI_CUBE_STEPS[offset % 6];
    return `rgb(${red}, ${green}, ${blue})`;
  }

  const grey = 8 + (paletteIndex - 232) * 10;
  return `rgb(${grey}, ${grey}, ${grey})`;
}

/// Anser reports a colour as one of its class names; `truecolor` holds the "r, g, b"
/// channels when that class is `ansi-truecolor`.
function colorForAnserClass(colorClass: string | null, truecolor: string | null): string | null {
  if (!colorClass) {
    return null;
  }
  if (colorClass === "ansi-truecolor") {
    return truecolor ? `rgb(${truecolor})` : null;
  }

  const paletteMatch = colorClass.match(/^ansi-palette-(\d+)$/);
  if (paletteMatch) {
    return paletteColor(Number(paletteMatch[1]));
  }

  return mapAnsiColorClassToColor[colorClass] ?? null;
}

// Anser leaves operating system commands (window titles and the like) in the text.
const OSC_PATTERN = /\x1b\][\s\S]*?(?:\x07|\x1b\\)/g;

// Everything the log surface cannot show: C0 controls other than newline and tab.
const CONTROL_CHARACTER_PATTERN = /[\x00-\x08\x0b-\x1f\x7f]/g;

export function parseAnsiSpans(value: string): AnsiSpan[] {
  const source = value.replace(/\r\n/g, "\n").replace(OSC_PATTERN, "");
  const spans: AnsiSpan[] = [];

  for (const chunk of Anser.ansiToJson(source, {
    json: true,
    remove_empty: true,
    use_classes: true,
  })) {
    const text = chunk.content.replace(CONTROL_CHARACTER_PATTERN, "");
    if (!text) {
      continue;
    }

    const style: AnsiStyle = {};
    const color = colorForAnserClass(chunk.fg, chunk.fg_truecolor);
    const backgroundColor = colorForAnserClass(chunk.bg, chunk.bg_truecolor);
    if (color) {
      style.color = color;
    }
    if (backgroundColor) {
      style.backgroundColor = backgroundColor;
    }
    for (const decoration of chunk.decorations) {
      Object.assign(style, mapDecorationToStyle[decoration]);
    }

    const previous = spans[spans.length - 1];
    if (previous && sameStyle(previous.style, style)) {
      previous.text += text;
      continue;
    }
    spans.push({ text, style });
  }

  return spans;
}

function sameStyle(left: AnsiStyle, right: AnsiStyle): boolean {
  return (
    left.color === right.color &&
    left.backgroundColor === right.backgroundColor &&
    left.fontWeight === right.fontWeight &&
    left.fontStyle === right.fontStyle &&
    left.textDecoration === right.textDecoration &&
    left.opacity === right.opacity
  );
}
