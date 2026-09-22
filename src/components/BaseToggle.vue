<script setup lang="ts">
const model = defineModel<boolean>({ required: true })

const props = withDefaults(defineProps<{
  disabled?: boolean
  size?: 'sm' | 'md'
  label?: string
  ariaLabel?: string
}>(), {
  disabled: false,
  size: 'md',
  label: '',
  ariaLabel: '',
})

const emit = defineEmits<{
  change: [value: boolean]
}>()

function onChange(event: Event) {
  if (props.disabled) return
  const target = event.target as HTMLInputElement
  model.value = target.checked
  emit('change', model.value)
}
</script>

<template>
  <label class="toggle-wrapper" :class="{ 'is-disabled': disabled }">
    <input
      type="checkbox"
      class="toggle-input"
      :checked="model"
      :disabled="disabled"
      role="switch"
      :aria-checked="model"
      :aria-label="ariaLabel || label || undefined"
      @change="onChange"
    />
    <span class="toggle-track" :class="[`size-${size}`, { 'is-on': model }]">
      <span class="toggle-knob" />
    </span>
    <span v-if="label" class="toggle-label">{{ label }}</span>
  </label>
</template>

<style scoped>
.toggle-wrapper {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  cursor: pointer;
  user-select: none;
  -webkit-user-select: none;
}

.toggle-wrapper.is-disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.toggle-input {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.toggle-input:focus-visible + .toggle-track {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.toggle-track {
  --toggle-width: 40px;
  --toggle-height: 22px;
  --toggle-knob-size: 16px;

  position: relative;
  width: var(--toggle-width);
  height: var(--toggle-height);
  background: var(--bg-hover);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-full);
  transition: all var(--duration-normal) var(--ease-out);
  flex-shrink: 0;
}

.toggle-track.size-sm {
  --toggle-width: 32px;
  --toggle-height: 18px;
  --toggle-knob-size: 12px;
}

.toggle-track.is-on {
  background: var(--accent);
  border-color: var(--accent);
}

.toggle-knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: calc(var(--toggle-knob-size));
  height: calc(var(--toggle-knob-size));
  background: white;
  border-radius: var(--radius-full);
  box-shadow: var(--shadow-xs);
  transition: all var(--duration-normal) var(--ease-spring);
  pointer-events: none;
}

.toggle-track.is-on .toggle-knob {
  left: calc(var(--toggle-width) - var(--toggle-knob-size) - 2px);
  box-shadow: var(--shadow-sm);
  animation: toggleOn var(--duration-normal) var(--ease-spring);
}

.toggle-track:not(.is-on) .toggle-knob {
  animation: toggleOff var(--duration-normal) var(--ease-spring);
}

.toggle-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  line-height: 1;
}
</style>
