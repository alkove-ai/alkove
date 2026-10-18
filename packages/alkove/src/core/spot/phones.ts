// the 58 phonemes Spot hears and Lex writes (index + 1 = model class, 0 = blank), and how other IPA folds onto them
export const PHONES = [
  'i', 'y', 'ɨ', 'u', 'e', 'ø', 'ə', 'o', 'ɛ', 'œ', 'ɔ', 'a', 'ɑ', 'æ', 'ʌ', 'ɪ', 'ʊ', 'ɐ', 'ɜ',
  'ɛ̃', 'ɑ̃', 'ɔ̃', 'ɐ̃', 'ẽ', 'ĩ', 'õ', 'ũ', 'j', 'w', 'ɥ', 'p', 'b', 't', 'd', 'k', 'ɡ', 'f', 'v',
  's', 'z', 'ʃ', 'ʒ', 'm', 'n', 'ɲ', 'ŋ', 'l', 'ʎ', 'ʁ', 'ɾ', 'r', 'ɹ', 'x', 'θ', 'ð', 'β', 'ɣ', 'h',
].map((p) => p.normalize('NFD'))

export const BLANK = 0
const ID = new Map(PHONES.map((p, i) => [p, i + 1]))
export const SCHWA = ID.get('ə')!

const COMMON: Record<string, string> = {
  'ᵻ': 'ɪ', 'ɚ': 'əɹ', 'ɝ': 'ɜɹ', 'ʝ': 'j', 'ɟ': 'ɡ', c: 'k', 'ɫ': 'l', 'ɬ': 'l', 'χ': 'x', 'ɦ': 'h', 'ʋ': 'v',
  'ɱ': 'm', 'ɻ': 'ɹ', 'ʀ': 'ʁ', g: 'ɡ', 'ɘ': 'ə', 'ɵ': 'ø', 'ʏ': 'y', 'ɯ': 'u', 'ɤ': 'o', q: 'k', 'ɕ': 'ʃ',
  'ʑ': 'ʒ', 'ç': 'ʃ', 'ʔ': '', 'ɒ': 'ɔ', 'œ̃': 'ɛ̃', 'ɸ': 'f', 'ʙ': 'b',
}
const NATIVE: Record<string, Record<string, string>> = {
  fr: {
    'ɑ': 'a', 'ã': 'ɑ̃', 'õ': 'ɔ̃', 'ẽ': 'ɛ̃', 'ĩ': 'ɛ̃', 'ũ': 'ɔ̃', 'ɐ̃': 'ɑ̃', 'ɐ': 'a', 'æ': 'a', 'ʌ': 'œ', 'ɜ': 'œ',
    'ɪ': 'i', 'ʊ': 'u', 'ɹ': 'ʁ', r: 'ʁ', 'ɾ': 'ʁ', x: 'ʁ', 'θ': 's', 'ð': 'z', h: '', 'β': 'b', 'ɣ': 'ɡ',
    'ʎ': 'lj', 'ɨ': 'i',
  },
  es: { 'ɪ': 'i', 'ʊ': 'u', 'ɛ': 'e', 'ɔ': 'o', 'ɑ': 'a', 'æ': 'a', 'ʌ': 'a', 'ə': 'e', 'ɐ': 'a', 'ɹ': 'ɾ' },
  it: { 'ɪ': 'i', 'ʊ': 'u', 'ɑ': 'a', 'æ': 'a', 'ʌ': 'a', 'ɐ': 'a', 'ɹ': 'r', x: 'k', 'θ': 't', h: '' },
  pt: { 'ɪ': 'i', 'ʊ': 'u', 'ʊ̃': 'ũ', y: 'j', 'ã': 'ɐ̃', 'ɑ': 'a', 'æ': 'a', 'ʌ': 'a', 'ɹ': 'ɾ' },
  ca: { 'ɪ': 'i', 'ʊ': 'u', 'ɑ': 'a', 'æ': 'a', 'ʌ': 'ə', 'ɹ': 'ɾ', 'ɐ': 'ə' },
  ro: { 'ɪ': 'i', 'ʊ': 'u', 'ɑ': 'a', 'æ': 'a', 'ʌ': 'ə', 'ɹ': 'r', 'ɾ': 'r' },
  en: { 'ɐ': 'ə', r: 'ɹ', x: 'k', 'ɛ̃': 'ɑ̃' },
}
const SKIP = new Set("ˈˌːˑ‿-.|ʲʰʷ' \t")

// acoustically interchangeable for the search: regional vowels, glide / vowel, tap / trill
const CLASSES = [['e', 'ɛ'], ['o', 'ɔ'], ['ø', 'œ', 'ə'], ['i', 'j'], ['u', 'w'], ['y', 'ɥ'], ['ɾ', 'r']]
export const SAME: number[][] = PHONES.map((_, i) => [i + 1])
for (const group of CLASSES) {
  const ids = group.map((p) => ID.get(p.normalize('NFD'))!)
  for (const i of ids) for (const j of ids) if (!SAME[i - 1]!.includes(j)) SAME[i - 1]!.push(j)
}

const folds = new Map<string, Map<string, string>>()
function fold(lang: string): Map<string, string> {
  let m = folds.get(lang)
  if (!m) {
    const entries = Object.entries({ ...COMMON, ...(NATIVE[lang] ?? {}) }).map(([k, v]) => [k.normalize('NFD'), v.normalize('NFD')] as const)
    folds.set(lang, (m = new Map(entries)))
  }
  return m
}

function tokens(ipa: string): string[] {
  const out: string[] = []
  for (const ch of ipa.normalize('NFD')) {
    if (/\p{M}/u.test(ch)) {
      if (ch === '̃' && out.length) out[out.length - 1] += ch
      continue
    }
    out.push(ch)
  }
  return out
}

// typed IPA -> classes, null if a symbol has no counterpart; ties and language flags « (en) » dropped
export function toIds(ipa: string, lang = ''): number[] | null {
  const m = fold(lang)
  const ids: number[] = []
  for (const t of tokens(ipa.replace(/\([^)]*\)/g, ' ').replace(/‍/g, '').replace(/\s+/g, ' '))) {
    if (SKIP.has(t)) continue
    for (let p of tokens(m.get(t) ?? t)) {
      p = m.get(p) ?? p
      const id = ID.get(p)
      if (!id) return null
      if (ids[ids.length - 1] !== id) ids.push(id)
    }
  }
  return ids
}

export const toIpa = (ids: readonly number[]) => ids.map((i) => PHONES[i - 1]).join('').normalize('NFC')
