<script setup lang="ts">
import AppSidebar from './components/AppSidebar.vue'
</script>

<template>
  <div class="app-shell">
    <AppSidebar />
    <main class="main-area">
      <div class="main-scroll">
        <router-view v-slot="{ Component }">
          <transition name="page" mode="out-in">
            <keep-alive>
              <component :is="Component" />
            </keep-alive>
          </transition>
        </router-view>
      </div>
    </main>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  height: 100vh;
  overflow: hidden;
  background: var(--bg-base);
}

.main-area {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-base);
}

.main-scroll {
  flex: 1;
  width: 100%;
  min-width: 0;
  box-sizing: border-box;
  overflow-y: auto;
  padding: var(--space-6) var(--space-8);
}

.page-enter-active {
  transition: opacity var(--duration-normal) var(--ease-out);
}
.page-enter-from { opacity: 0; }

@media (max-width: 700px) {
  .main-scroll {
    padding: var(--space-4);
  }

}
</style>
