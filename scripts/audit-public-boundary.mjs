// Public-boundary audit for the WinOptimizer Free repository.
//
// Fails when the public tree contains commercial implementation identity or
// algorithm signatures. Product vocabulary ("Duplicate Finder",
// "WinOptimizer Pro") is allowed.
//
// The forbidden identifiers are assembled at runtime so this checker does not
// itself contain the strings it forbids.

import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const SKIP = new Set(['.git', 'node_modules', 'target', '.build', 'dist'])
const BINARY = new Set(['.png', '.jpg', '.jpeg', '.gif', '.ico', '.webp', '.woff', '.woff2', '.ttf', '.otf', '.pdf', '.zip', '.gz', '.7z', '.exe', '.dll', '.pdb', '.msi'])
const SELF = new Set(['scripts/audit-public-boundary.mjs', 'scripts/verify-public.ps1'])

const fragments = [
  ['winoptimizer', 'dedup'], ['winoptimizer', 'pro'], ['winoptimizer', 'commercial'],
  ['media', 'fingerprint'], ['media', 'preview'], ['media', 'foundation'],
  ['duplicate', 'operation'], ['duplicate', 'review'], ['duplicate', 'cache'],
  ['media', 'duplicate', 'cleanup'], ['find', 'duplicate', 'files'],
  ['generate', 'duplicate', 'preview'], ['dedup', 'pro'],
]
const identity = fragments.map((parts) => parts.join(['winoptimizer', 'dedup'].includes(parts[0]) ? '-' : '_'))
const signatures = [
  ['perceptual', 'hash'].join(' '),
  ['audio', 'fingerprint'].join('_'),
  ['video', 'fingerprint'].join('_'),
  ['image', 'phash'].join('_'),
  ['dedup', 'engine'].join(' '),
  ['fingerprint', 'database'].join(' '),
  ['similarity', 'engine'].join(' '),
]
const tokens = [...identity, ...signatures]

const files = []
const walk = (directory) => {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (SKIP.has(entry.name)) continue
    const full = join(directory, entry.name)
    if (entry.isDirectory()) walk(full)
    else if (entry.isFile()) files.push(full)
  }
}
walk(ROOT)

let scanned = 0
const findings = []
for (const file of files) {
  const rel = relative(ROOT, file).split('\\').join('/')
  if (SELF.has(rel)) continue
  const dot = file.lastIndexOf('.')
  if (dot >= 0 && BINARY.has(file.slice(dot).toLowerCase())) continue
  let content
  try {
    content = readFileSync(file, 'utf8')
  } catch {
    continue
  }
  scanned += 1
  const folded = content.toLowerCase()
  for (const token of tokens) {
    const at = folded.indexOf(token.toLowerCase())
    if (at >= 0) findings.push({ rel, token, line: folded.slice(0, at).split('\n').length })
  }
}

if (findings.length > 0) {
  for (const finding of findings) {
    console.error(`public boundary violation ${finding.rel}:${finding.line}: ${finding.token}`)
  }
  process.exitCode = 1
} else {
  console.log(`public boundary audit passed (${scanned} text files scanned)`)
}
