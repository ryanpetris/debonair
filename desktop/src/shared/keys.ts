// Physical keys, named by their DOM KeyboardEvent.code, and the HID usages a Target receives for
// them. Keys are physical positions, so the Target applies its own keyboard layout.

/** A key on the HID keyboard page, or a control on the consumer page. */
export interface KeyUsage {
  page: "keyboard" | "consumer";
  usage: number;
  /** Short label for buttons and shortcut text. */
  label: string;
}

const keyboard = (usage: number, label: string): KeyUsage => ({ page: "keyboard", usage, label });
const consumer = (usage: number, label: string): KeyUsage => ({ page: "consumer", usage, label });

const table: Record<string, KeyUsage> = {};
for (let i = 0; i < 26; i++) {
  const letter = String.fromCharCode(65 + i);
  table[`Key${letter}`] = keyboard(0x04 + i, letter);
}
for (let i = 1; i <= 9; i++) table[`Digit${i}`] = keyboard(0x1d + i, String(i));
table.Digit0 = keyboard(0x27, "0");
for (let i = 1; i <= 12; i++) table[`F${i}`] = keyboard(0x39 + i, `F${i}`);
for (let i = 13; i <= 24; i++) table[`F${i}`] = keyboard(0x68 + i - 13, `F${i}`);
for (let i = 1; i <= 9; i++) table[`Numpad${i}`] = keyboard(0x58 + i, `Num ${i}`);
table.Numpad0 = keyboard(0x62, "Num 0");

Object.assign(table, {
  Enter: keyboard(0x28, "Enter"),
  Escape: keyboard(0x29, "Esc"),
  Backspace: keyboard(0x2a, "Backspace"),
  Tab: keyboard(0x2b, "Tab"),
  Space: keyboard(0x2c, "Space"),
  Minus: keyboard(0x2d, "-"),
  Equal: keyboard(0x2e, "="),
  BracketLeft: keyboard(0x2f, "["),
  BracketRight: keyboard(0x30, "]"),
  Backslash: keyboard(0x31, "\\"),
  IntlHash: keyboard(0x32, "#"),
  Semicolon: keyboard(0x33, ";"),
  Quote: keyboard(0x34, "'"),
  Backquote: keyboard(0x35, "`"),
  Comma: keyboard(0x36, ","),
  Period: keyboard(0x37, "."),
  Slash: keyboard(0x38, "/"),
  CapsLock: keyboard(0x39, "Caps Lock"),
  PrintScreen: keyboard(0x46, "Print Screen"),
  ScrollLock: keyboard(0x47, "Scroll Lock"),
  Pause: keyboard(0x48, "Pause"),
  Insert: keyboard(0x49, "Insert"),
  Home: keyboard(0x4a, "Home"),
  PageUp: keyboard(0x4b, "Page Up"),
  Delete: keyboard(0x4c, "Delete"),
  End: keyboard(0x4d, "End"),
  PageDown: keyboard(0x4e, "Page Down"),
  ArrowRight: keyboard(0x4f, "Right"),
  ArrowLeft: keyboard(0x50, "Left"),
  ArrowDown: keyboard(0x51, "Down"),
  ArrowUp: keyboard(0x52, "Up"),
  NumLock: keyboard(0x53, "Num Lock"),
  NumpadDivide: keyboard(0x54, "Num /"),
  NumpadMultiply: keyboard(0x55, "Num *"),
  NumpadSubtract: keyboard(0x56, "Num -"),
  NumpadAdd: keyboard(0x57, "Num +"),
  NumpadEnter: keyboard(0x58, "Num Enter"),
  NumpadDecimal: keyboard(0x63, "Num ."),
  IntlBackslash: keyboard(0x64, "\\"),
  ContextMenu: keyboard(0x65, "Menu"),
  Power: keyboard(0x66, "Power"),
  NumpadEqual: keyboard(0x67, "Num ="),
  Help: keyboard(0x75, "Help"),
  Undo: keyboard(0x7a, "Undo"),
  Cut: keyboard(0x7b, "Cut"),
  Copy: keyboard(0x7c, "Copy"),
  Paste: keyboard(0x7d, "Paste"),
  Find: keyboard(0x7e, "Find"),
  NumpadComma: keyboard(0x85, "Num ,"),
  IntlRo: keyboard(0x87, "Ro"),
  KanaMode: keyboard(0x88, "Kana"),
  IntlYen: keyboard(0x89, "Yen"),
  Convert: keyboard(0x8a, "Convert"),
  NonConvert: keyboard(0x8b, "Non-Convert"),
  Lang1: keyboard(0x90, "Lang 1"),
  Lang2: keyboard(0x91, "Lang 2"),
  ControlLeft: keyboard(0xe0, "Ctrl"),
  ShiftLeft: keyboard(0xe1, "Shift"),
  AltLeft: keyboard(0xe2, "Alt"),
  MetaLeft: keyboard(0xe3, "Super"),
  ControlRight: keyboard(0xe4, "Right Ctrl"),
  ShiftRight: keyboard(0xe5, "Right Shift"),
  AltRight: keyboard(0xe6, "Right Alt"),
  MetaRight: keyboard(0xe7, "Right Super"),
  AudioVolumeMute: consumer(0xe2, "Mute"),
  AudioVolumeDown: consumer(0xea, "Volume Down"),
  AudioVolumeUp: consumer(0xe9, "Volume Up"),
  MediaPlayPause: consumer(0xcd, "Play/Pause"),
  MediaStop: consumer(0xb7, "Stop"),
  MediaTrackNext: consumer(0xb5, "Next Track"),
  MediaTrackPrevious: consumer(0xb6, "Previous Track"),
  MediaSelect: consumer(0x183, "Media"),
  Eject: consumer(0xb8, "Eject"),
  BrightnessUp: consumer(0x6f, "Brightness Up"),
  BrightnessDown: consumer(0x70, "Brightness Down"),
  LaunchMail: consumer(0x18a, "Mail"),
  LaunchApp1: consumer(0x194, "My Computer"),
  LaunchApp2: consumer(0x192, "Calculator"),
  BrowserSearch: consumer(0x221, "Search"),
  BrowserHome: consumer(0x223, "Browser Home"),
  BrowserBack: consumer(0x224, "Back"),
  BrowserForward: consumer(0x225, "Forward"),
  BrowserStop: consumer(0x226, "Browser Stop"),
  BrowserRefresh: consumer(0x227, "Refresh"),
  BrowserFavorites: consumer(0x22a, "Favorites"),
  Sleep: consumer(0x32, "Sleep"),
});

/** Every key the app can send, by DOM code. */
export const KEYS: Readonly<Record<string, KeyUsage>> = table;

export function keyUsage(code: string): KeyUsage | undefined {
  return Object.hasOwn(KEYS, code) ? KEYS[code] : undefined;
}

/** The HID modifier bit for a modifier key, or 0. */
export function modifierBit(code: string): number {
  const usage = keyUsage(code);
  return usage && usage.page === "keyboard" && usage.usage >= 0xe0 && usage.usage <= 0xe7 ? 1 << (usage.usage - 0xe0) : 0;
}

export function isModifier(code: string) {
  return modifierBit(code) !== 0;
}

/** The four modifier kinds, independent of side. */
export type Modifier = "ctrl" | "alt" | "shift" | "meta";

export function modifierKind(code: string): Modifier | null {
  if (code.startsWith("Control")) return "ctrl";
  if (code.startsWith("Alt")) return "alt";
  if (code.startsWith("Shift")) return "shift";
  if (code.startsWith("Meta")) return "meta";
  return null;
}

export function keyLabel(code: string) {
  return keyUsage(code)?.label ?? code;
}

/** Keys of a US layout that type each printable ASCII character, with Shift where needed. */
const typed: Record<string, [string, boolean]> = { " ": ["Space", false], "\n": ["Enter", false], "\t": ["Tab", false] };
for (let i = 0; i < 26; i++) {
  const letter = String.fromCharCode(97 + i);
  typed[letter] = [`Key${letter.toUpperCase()}`, false];
  typed[letter.toUpperCase()] = [`Key${letter.toUpperCase()}`, true];
}
const digits = ")!@#$%^&*(";
for (let i = 0; i < 10; i++) {
  typed[String(i)] = [`Digit${i}`, false];
  typed[digits[i]!] = [`Digit${i}`, true];
}
for (const [plain, shifted, code] of [
  ["-", "_", "Minus"],
  ["=", "+", "Equal"],
  ["[", "{", "BracketLeft"],
  ["]", "}", "BracketRight"],
  ["\\", "|", "Backslash"],
  [";", ":", "Semicolon"],
  ["'", '"', "Quote"],
  ["`", "~", "Backquote"],
  [",", "<", "Comma"],
  [".", ">", "Period"],
  ["/", "?", "Slash"],
] as const) {
  typed[plain] = [code, false];
  typed[shifted] = [code, true];
}

/** The key and Shift state that type `character` on a US layout, or null when there is none. */
export function keyForCharacter(character: string): { code: string; shift: boolean } | null {
  const entry = Object.hasOwn(typed, character) ? typed[character] : undefined;
  return entry ? { code: entry[0], shift: entry[1] } : null;
}
