import { createApp } from 'vue'
import { createPinia } from 'pinia'
import router from './router'
import App from './App.vue'
import './assets/design-tokens.css'
import { executeResumedOperation, hasResumeOperationArgs } from './api/operation'
import { isTauri } from '@tauri-apps/api/core'

const app = createApp(App)
app.use(createPinia())
app.use(router)

router.isReady().then(async () => {
  if (!isTauri()) {
    app.mount('#app')
    return
  }
  if (await hasResumeOperationArgs()) {
    try {
      const record = await executeResumedOperation()
      if (record) {
        await router.replace({ name: 'History', query: { operation: record.operation_id, resumed: 'elevation' } })
      }
    } catch {
      await router.replace({ name: 'History', query: { resumed: 'elevation-failed' } })
    }
  }
  app.mount('#app')
})
