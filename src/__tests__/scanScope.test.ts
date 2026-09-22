import { describe, expect, it } from 'vitest'
import { describeScanScope, isVolumeRootPath } from '../utils/scanScope'

describe('isVolumeRootPath', () => {
  it('recognizes drive roots with either separator', () => {
    expect(isVolumeRootPath('C:\\')).toBe(true)
    expect(isVolumeRootPath('D:/')).toBe(true)
    expect(isVolumeRootPath('  C:\\  ')).toBe(true)
  })

  it('rejects subtrees and partial drive strings', () => {
    expect(isVolumeRootPath('C:\\Users\\Alice')).toBe(false)
    expect(isVolumeRootPath('C:')).toBe(false)
    expect(isVolumeRootPath('C:\\Users')).toBe(false)
    expect(isVolumeRootPath('')).toBe(false)
  })
})

describe('describeScanScope', () => {
  it('reports an entire volume only for an exact drive root', () => {
    expect(describeScanScope('C:\\')).toBe('Entire volume')
    expect(describeScanScope('E:/')).toBe('Entire volume')
  })

  it('reports a selected folder for a subtree the backend walks with Win32', () => {
    expect(describeScanScope('C:\\Users\\Alice')).toBe('Selected folder')
    expect(describeScanScope('C:\\Windows\\Logs')).toBe('Selected folder')
  })
})
