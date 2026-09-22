import { describe, expect, it } from 'vitest'
import {
  getTauriErrorCode,
  isTauriErrorCode,
  OPERATION_CANCELLED_ERROR_CODE,
} from '../api/tauri'

describe('Tauri IPC error helpers', () => {
  it('reads stable codes from generic and structured operation errors', () => {
    const cancellation = {
      code: OPERATION_CANCELLED_ERROR_CODE,
      category: 'cancellation',
      operation_id: 'preview-1',
      phase: 'cancellation',
      message: 'Operation cancelled by user',
      human_message: 'Operation cancelled by user',
      retryable: true,
    }

    expect(getTauriErrorCode(cancellation)).toBe('operation_cancelled')
    expect(isTauriErrorCode(cancellation, OPERATION_CANCELLED_ERROR_CODE)).toBe(true)
    expect(isTauriErrorCode('Operation cancelled by user', OPERATION_CANCELLED_ERROR_CODE)).toBe(false)
  })
})
