<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue'
import type { OperationPlan } from '../api/operation'
import BaseButton from './BaseButton.vue'
import BaseBadge from './BaseBadge.vue'
import OperationSafetySummary from './OperationSafetySummary.vue'

interface ConfirmItem {
  label: string
  value?: string
  risk?: 'low' | 'medium' | 'high' | 'info'
}

const props = withDefaults(defineProps<{
  open: boolean
  title: string
  description?: string
  items?: ConfirmItem[]
  confirmLabel?: string
  cancelLabel?: string
  variant?: 'accent' | 'danger' | 'warning' | 'primary'
  loading?: boolean
  icon?: string
  operationPlan?: OperationPlan | null
}>(), {
  open: false,
  description: '',
  items: () => [],
  confirmLabel: 'Confirm',
  cancelLabel: 'Cancel',
  variant: 'primary',
  loading: false,
  icon: '',
  operationPlan: null,
})

const emit = defineEmits<{
  confirm: []
  cancel: []
  'update:open': [value: boolean]
}>()

const dialogRef = ref<HTMLElement | null>(null)
let previousFocus: HTMLElement | null = null

function onBackdropClick(e: MouseEvent) {
  if (e.target === e.currentTarget) {
    onCancel()
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && props.open) {
    onCancel()
    return
  }
  // Focus trap: Tab cycles within dialog
  if (e.key === 'Tab' && dialogRef.value) {
    const focusable = dialogRef.value.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    )
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    if (e.shiftKey) {
      if (document.activeElement === first) {
        e.preventDefault()
        last.focus()
      }
    } else {
      if (document.activeElement === last) {
        e.preventDefault()
        first.focus()
      }
    }
  }
}

function focusDialog() {
  nextTick(() => {
    if (!dialogRef.value) return
    // Focus the first focusable element (cancel button)
    const focusable = dialogRef.value.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    )
    if (focusable.length > 0) {
      focusable[0].focus()
    } else {
      dialogRef.value.focus()
    }
  })
}

watch(() => props.open, (val) => {
  if (val) {
    previousFocus = document.activeElement as HTMLElement
    document.addEventListener('keydown', onKeydown)
    focusDialog()
  } else {
    document.removeEventListener('keydown', onKeydown)
    // Return focus to the element that opened the dialog
    if (previousFocus) {
      nextTick(() => previousFocus?.focus())
    }
  }
})

onMounted(() => {
  if (props.open) {
    document.addEventListener('keydown', onKeydown)
  }
})

onUnmounted(() => {
  document.removeEventListener('keydown', onKeydown)
})

function onConfirm() {
  emit('confirm')
}

function onCancel() {
  emit('cancel')
  emit('update:open', false)
}

function riskVariant(risk?: string) {
  switch (risk) {
    case 'low': return 'success'
    case 'medium': return 'warning'
    case 'high': return 'danger'
    case 'info': return 'info'
    default: return 'default'
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="open" class="modal-backdrop" @click="onBackdropClick">
        <div ref="dialogRef" class="modal-dialog" role="dialog" aria-modal="true" :aria-label="title" tabindex="-1">
          <!-- Header -->
          <div class="modal-header">
            <div class="modal-title-row">
              <svg v-if="icon" class="modal-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="22" height="22" aria-hidden="true">
                <template v-if="icon === 'trash'">
                  <path d="M3 6h18" /><path d="M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2" /><path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6" />
                </template>
                <template v-else-if="icon === 'shield'">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
                </template>
                <template v-else-if="icon === 'zap'">
                  <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
                </template>
                <template v-else-if="icon === 'x-circle'">
                  <circle cx="12" cy="12" r="10" /><path d="M15 9l-6 6" /><path d="M9 9l6 6" />
                </template>
                <template v-else>
                  <circle cx="12" cy="12" r="10" /><path d="M12 16v-4" /><path d="M12 8h.01" />
                </template>
              </svg>
              <h3 class="modal-title">{{ title }}</h3>
              <BaseBadge v-if="variant === 'danger'" variant="danger" size="sm" pulse>
                Destructive
              </BaseBadge>
            </div>
            <button type="button" class="modal-close" @click="onCancel" aria-label="Close dialog">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16" aria-hidden="true">
                <path d="M18 6L6 18" /><path d="M6 6l12 12" />
              </svg>
            </button>
          </div>

          <!-- Description -->
          <p v-if="description" class="modal-description">{{ description }}</p>

          <OperationSafetySummary
            v-if="operationPlan"
            :plan="operationPlan"
            title="Safety summary"
            compact
          />

          <!-- Items list -->
          <div v-if="items.length > 0" class="modal-items">
            <div
              v-for="(item, idx) in items"
              :key="idx"
              class="modal-item"
            >
              <div class="modal-item-info">
                <span class="modal-item-label">{{ item.label }}</span>
                <span v-if="item.value" class="modal-item-value">{{ item.value }}</span>
              </div>
              <BaseBadge
                v-if="item.risk"
                :variant="riskVariant(item.risk)"
                size="sm"
                dot
              >
                {{ item.risk }}
              </BaseBadge>
            </div>
          </div>

          <!-- Actions -->
          <div class="modal-actions">
            <BaseButton variant="ghost" size="md" @click="onCancel" :disabled="loading">
              {{ cancelLabel }}
            </BaseButton>
            <BaseButton
              :variant="variant === 'danger' ? 'danger' : variant === 'warning' ? 'warning' : 'primary'"
              size="md"
              @click="onConfirm"
              :loading="loading"
              icon="check"
            >
              {{ confirmLabel }}
            </BaseButton>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* Backdrop */
.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(25, 27, 37, .32);
}

/* Dialog */
.modal-dialog {
  width: 480px;
  max-width: 90vw;
  max-height: 80vh;
  overflow-y: auto;
  background: var(--bg-elevated);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-2xl);
  box-shadow: var(--shadow-overlay);
  padding: var(--space-6);
  animation: dialogIn 0.2s var(--ease-out);
}

/* Header */
.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: var(--space-4);
}

.modal-title-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex: 1;
  min-width: 0;
}

.modal-icon {
  color: var(--accent);
  flex-shrink: 0;
}

.modal-title {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  line-height: 1.3;
}

.modal-close {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
  color: var(--text-muted);
  cursor: pointer;
  transition: all var(--duration-fast);
  flex-shrink: 0;
}

.modal-close:hover {
  background: var(--glass-bg-hover);
  color: var(--text-primary);
  border-color: var(--glass-border-hover);
}

/* Description */
.modal-description {
  margin: 0 0 var(--space-5);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  line-height: var(--leading-relaxed);
}

/* Items list */
.modal-items {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin-bottom: var(--space-6);
  max-height: 300px;
  overflow-y: auto;
}

.modal-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  background: var(--bg-surface);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
}

.modal-item-info {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1;
}

.modal-item-label {
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.modal-item-value {
  font-size: var(--text-xs);
  color: var(--text-muted);
  font-family: var(--font-mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Actions */
.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-3);
}

/* Transitions */
.modal-enter-active {
  transition: opacity 0.2s var(--ease-out);
}
.modal-leave-active {
  transition: opacity 0.15s var(--ease-in);
}
.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

@keyframes dialogIn {
  from {
    opacity: 0;
    transform: scale(0.95) translateY(8px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}
</style>
