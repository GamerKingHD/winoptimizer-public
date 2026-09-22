import type { OperationError } from '../api/operation'

export type BannerKind = 'success' | 'error' | 'warning' | 'info'

export interface ActionFeedback {
  type: BannerKind
  title: string
  message: string
}

export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  if (error && typeof error === 'object' && 'message' in error) {
    return String((error as { message: unknown }).message)
  }
  return 'The action failed. No additional details were returned.'
}

export function failure(title: string, error: unknown): ActionFeedback {
  return {
    type: 'error',
    title,
    message: errorMessage(error),
  }
}

export function success(title: string, message: string): ActionFeedback {
  return { type: 'success', title, message }
}

export function isElevationError(error: unknown): boolean {
  if (!error || typeof error !== 'object') return false
  const structured = error as Partial<OperationError>
  return structured.code === 'elevation_required' || structured.requires_elevation === true
}
