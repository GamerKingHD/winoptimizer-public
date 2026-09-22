import { describe, expect, it } from 'vitest'
import { formatBytes, formatPercent } from '../utils/format'

describe('formatPercent', () => {
  it('rounds to the nearest whole percent', () => {
    expect(formatPercent(30.256342961305194)).toBe('30%')
    expect(formatPercent(30.5)).toBe('31%')
  })
})

describe('formatBytes', () => {
  it('labels the 1024-based divisor with binary units', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(512)).toBe('512.0 B')
    expect(formatBytes(1024)).toBe('1.0 KiB')
    expect(formatBytes(1024 * 1024)).toBe('1.0 MiB')
    expect(formatBytes(5 * 1024 * 1024 * 1024)).toBe('5.0 GiB')
  })
})
