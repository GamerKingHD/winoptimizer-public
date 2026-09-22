import { describe, it, expect } from 'vitest'
import { errorMessage, failure, success, isElevationError } from '../utils/feedback'

describe('feedback utils', () => {
  it('errorMessage extracts string directly', () => {
    expect(errorMessage('oops')).toBe('oops')
  })

  it('errorMessage extracts Error.message', () => {
    expect(errorMessage(new Error('fail'))).toBe('fail')
  })

  it('errorMessage extracts object.message', () => {
    expect(errorMessage({ message: 'custom' })).toBe('custom')
  })

  it('errorMessage returns fallback for unknown', () => {
    expect(errorMessage(42)).toBe('The action failed. No additional details were returned.')
  })

  it('failure builds ActionFeedback with error type', () => {
    const fb = failure('Title', 'details')
    expect(fb).toEqual({ type: 'error', title: 'Title', message: 'details' })
  })

  it('success builds ActionFeedback with success type', () => {
    const fb = success('Done', 'ok')
    expect(fb).toEqual({ type: 'success', title: 'Done', message: 'ok' })
  })

  it('isElevationError uses structured error fields', () => {
    expect(isElevationError({ code: 'elevation_required', message: 'Administrator approval needed', retryable: true })).toBe(true)
    expect(isElevationError({ code: 'execution_failed', requires_elevation: true })).toBe(true)
    expect(isElevationError('Elevation required')).toBe(false)
    expect(isElevationError({ code: 'execution_failed', message: 'Elevation required' })).toBe(false)
  })
})
