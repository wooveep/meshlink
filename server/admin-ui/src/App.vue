<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'

type ServiceEnvelope = {
  service: string
  healthy: boolean
  now: string
  error?: string
  status?: Record<string, unknown>
}

type AuditEvent = {
  id: number
  occurred_at: string
  actor: string
  action: string
  device_id?: string
  summary: string
}

type Device = {
  id: string
  name: string
  public_key: string
  os: string
  version: string
  overlay_ip: string
  direct_endpoint?: { host: string; port: number }
  advertised_routes: string[]
  labels: Record<string, string>
  disabled: boolean
  last_seen: string
  online: boolean
}

type DeviceDetail = {
  device: Device
  peers: Array<{
    peer_id: string
    public_key: string
    overlay_ip: string
    allowed_ips: string[]
    direct_endpoint?: { host: string; port: number }
  }>
}

type Overview = {
  generated_at: string
  management: {
    revision: string
    device_total: number
    online_devices: number
    offline_devices: number
  }
  services: ServiceEnvelope[]
  recent_events: AuditEvent[]
}

type ConfigSummary = {
  grpc_listen_addr: string
  http_listen_addr: string
  overlay_cidr: string
  sync_interval: string
  state_db_path: string
  admin_token_configured: boolean
  signald_status_url: string
  relayd_status_url: string
}

const view = ref<'overview' | 'devices' | 'services' | 'audit'>('overview')
const authenticated = ref(false)
const loading = ref(false)
const errorMessage = ref('')
const saving = ref(false)
const loginToken = ref('')
const search = ref('')
const selectedDeviceId = ref('')

const overview = ref<Overview | null>(null)
const devices = ref<Device[]>([])
const deviceDetail = ref<DeviceDetail | null>(null)
const services = ref<ServiceEnvelope[]>([])
const config = ref<ConfigSummary | null>(null)
const audit = ref<AuditEvent[]>([])

const editState = reactive({
  name: '',
  labelsText: '',
  routesText: ''
})

const filteredDevices = computed(() => {
  const query = search.value.trim().toLowerCase()
  if (!query) {
    return devices.value
  }
  return devices.value.filter((device) =>
    [device.name, device.id, device.overlay_ip, device.os, device.version]
      .join(' ')
      .toLowerCase()
      .includes(query)
  )
})

const selectedDevice = computed(() => deviceDetail.value?.device ?? null)

watch(selectedDevice, (device) => {
  if (!device) {
    editState.name = ''
    editState.labelsText = ''
    editState.routesText = ''
    return
  }
  editState.name = device.name
  editState.labelsText = Object.entries(device.labels)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')
  editState.routesText = device.advertised_routes.join('\n')
})

onMounted(async () => {
  try {
    await refreshAll()
    authenticated.value = true
  } catch {
    authenticated.value = false
  }
})

async function refreshAll() {
  loading.value = true
  errorMessage.value = ''
  try {
    const [overviewResponse, deviceResponse, serviceResponse, configResponse, auditResponse] = await Promise.all([
      apiGet<Overview>('/api/admin/v1/overview'),
      apiGet<{ devices: Device[] }>('/api/admin/v1/devices'),
      apiGet<{ services: ServiceEnvelope[] }>('/api/admin/v1/services'),
      apiGet<ConfigSummary>('/api/admin/v1/config'),
      apiGet<{ events: AuditEvent[] }>('/api/admin/v1/audit')
    ])

    overview.value = overviewResponse
    devices.value = deviceResponse.devices
    services.value = serviceResponse.services
    config.value = configResponse
    audit.value = auditResponse.events

    if (!selectedDeviceId.value && deviceResponse.devices.length > 0) {
      selectedDeviceId.value = deviceResponse.devices[0].id
    }
    if (selectedDeviceId.value) {
      await loadDeviceDetail(selectedDeviceId.value)
    }
  } finally {
    loading.value = false
  }
}

async function loadDeviceDetail(deviceId: string) {
  selectedDeviceId.value = deviceId
  deviceDetail.value = await apiGet<DeviceDetail>(`/api/admin/v1/devices/${deviceId}`)
}

async function login() {
  loading.value = true
  errorMessage.value = ''
  try {
    await apiPost('/api/admin/v1/session/login', { token: loginToken.value })
    authenticated.value = true
    loginToken.value = ''
    await refreshAll()
  } catch (error) {
    errorMessage.value = getErrorMessage(error)
    authenticated.value = false
  } finally {
    loading.value = false
  }
}

async function logout() {
  await apiPost('/api/admin/v1/session/logout', {})
  authenticated.value = false
}

async function saveDevice() {
  if (!selectedDevice.value) return
  saving.value = true
  errorMessage.value = ''
  try {
    const labels = parseLabels(editState.labelsText)
    const advertisedRoutes = editState.routesText
      .split('\n')
      .map((route) => route.trim())
      .filter(Boolean)

    await apiPatch(`/api/admin/v1/devices/${selectedDevice.value.id}`, {
      name: editState.name,
      labels,
      advertised_routes: advertisedRoutes
    })
    await refreshAll()
  } catch (error) {
    errorMessage.value = getErrorMessage(error)
  } finally {
    saving.value = false
  }
}

async function toggleDevice(disabled: boolean) {
  if (!selectedDevice.value) return
  saving.value = true
  errorMessage.value = ''
  try {
    await apiPost(`/api/admin/v1/devices/${selectedDevice.value.id}/${disabled ? 'disable' : 'enable'}`, {})
    await refreshAll()
  } catch (error) {
    errorMessage.value = getErrorMessage(error)
  } finally {
    saving.value = false
  }
}

async function deleteDevice() {
  if (!selectedDevice.value) return
  const currentId = selectedDevice.value.id
  saving.value = true
  errorMessage.value = ''
  try {
    await apiDelete(`/api/admin/v1/devices/${currentId}`)
    selectedDeviceId.value = ''
    deviceDetail.value = null
    await refreshAll()
  } catch (error) {
    errorMessage.value = getErrorMessage(error)
  } finally {
    saving.value = false
  }
}

function parseLabels(input: string) {
  const labels: Record<string, string> = {}
  for (const line of input.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    const separator = trimmed.indexOf('=')
    if (separator <= 0) {
      throw new Error(`Invalid label entry: ${trimmed}`)
    }
    const key = trimmed.slice(0, separator).trim()
    const value = trimmed.slice(separator + 1).trim()
    labels[key] = value
  }
  return labels
}

async function apiGet<T>(url: string): Promise<T> {
  const response = await fetch(url, { credentials: 'include' })
  if (!response.ok) {
    throw await toError(response)
  }
  return response.json() as Promise<T>
}

async function apiPost(url: string, body: unknown) {
  const response = await fetch(url, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  })
  if (!response.ok) {
    throw await toError(response)
  }
  return response.json()
}

async function apiPatch(url: string, body: unknown) {
  const response = await fetch(url, {
    method: 'PATCH',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  })
  if (!response.ok) {
    throw await toError(response)
  }
  return response.json()
}

async function apiDelete(url: string) {
  const response = await fetch(url, {
    method: 'DELETE',
    credentials: 'include'
  })
  if (!response.ok) {
    throw await toError(response)
  }
  return response.json()
}

async function toError(response: Response) {
  let message = `${response.status} ${response.statusText}`
  try {
    const payload = (await response.json()) as { error?: string }
    if (payload.error) {
      message = payload.error
    }
  } catch {
    // Keep the transport error when the payload is not JSON.
  }
  return new Error(message)
}

function getErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Unexpected error'
}
</script>

<template>
  <div class="shell">
    <div class="ambient ambient-left"></div>
    <div class="ambient ambient-right"></div>

    <template v-if="!authenticated">
      <main class="login-panel">
        <div class="login-copy">
          <p class="eyebrow">MeshLink Server Console</p>
          <h1>Control the mesh without touching the shell.</h1>
          <p class="lede">
            One cockpit for managementd, signald, and relayd. Observe sessions, tune
            topology metadata, and keep the network honest.
          </p>
        </div>

        <form class="login-card" @submit.prevent="login">
          <label class="field">
            <span>Admin token</span>
            <input v-model="loginToken" type="password" autocomplete="current-password" />
          </label>
          <button class="primary-button" :disabled="loading || !loginToken.trim()">
            {{ loading ? 'Signing in…' : 'Enter Console' }}
          </button>
          <p v-if="errorMessage" class="error-banner">{{ errorMessage }}</p>
        </form>
      </main>
    </template>

    <template v-else>
      <div class="frame">
        <aside class="sidebar">
          <div>
            <p class="eyebrow">MeshLink</p>
            <h1>Server Atlas</h1>
            <p class="sidebar-copy">Topology, sessions, routes, and audit in one view.</p>
          </div>

          <nav class="nav">
            <button :class="['nav-item', { active: view === 'overview' }]" @click="view = 'overview'">
              Overview
            </button>
            <button :class="['nav-item', { active: view === 'devices' }]" @click="view = 'devices'">
              Devices
            </button>
            <button :class="['nav-item', { active: view === 'services' }]" @click="view = 'services'">
              Services
            </button>
            <button :class="['nav-item', { active: view === 'audit' }]" @click="view = 'audit'">
              Audit
            </button>
          </nav>

          <div class="sidebar-actions">
            <button class="ghost-button" @click="refreshAll" :disabled="loading">
              {{ loading ? 'Refreshing…' : 'Refresh data' }}
            </button>
            <button class="ghost-button" @click="logout">Sign out</button>
          </div>
        </aside>

        <main class="content">
          <header class="masthead">
            <div>
              <p class="eyebrow">Unified Operations View</p>
              <h2>{{ view.charAt(0).toUpperCase() + view.slice(1) }}</h2>
            </div>
            <p v-if="overview" class="timestamp">
              Last sync {{ new Date(overview.generated_at).toLocaleString() }}
            </p>
          </header>

          <p v-if="errorMessage" class="error-banner">{{ errorMessage }}</p>

          <section v-if="view === 'overview'" class="panel-stack">
            <div class="stat-grid">
              <article class="stat-card">
                <span>Total devices</span>
                <strong>{{ overview?.management.device_total ?? 0 }}</strong>
              </article>
              <article class="stat-card">
                <span>Online</span>
                <strong>{{ overview?.management.online_devices ?? 0 }}</strong>
              </article>
              <article class="stat-card">
                <span>Offline</span>
                <strong>{{ overview?.management.offline_devices ?? 0 }}</strong>
              </article>
              <article class="stat-card">
                <span>Revision</span>
                <strong>{{ overview?.management.revision ?? 'n/a' }}</strong>
              </article>
            </div>

            <div class="two-column">
              <article class="panel">
                <div class="panel-title">
                  <h3>Service health</h3>
                  <p>Live status from managementd, signald, and relayd.</p>
                </div>
                <div class="service-list">
                  <div v-for="service in services" :key="service.service" class="service-card">
                    <div class="service-heading">
                      <strong>{{ service.service }}</strong>
                      <span :class="['pill', service.healthy ? 'healthy' : 'degraded']">
                        {{ service.healthy ? 'healthy' : 'degraded' }}
                      </span>
                    </div>
                    <pre class="service-status">{{ JSON.stringify(service.status ?? {}, null, 2) }}</pre>
                    <p v-if="service.error" class="service-error">{{ service.error }}</p>
                  </div>
                </div>
              </article>

              <article class="panel">
                <div class="panel-title">
                  <h3>Recent events</h3>
                  <p>Most recent admin and system actions persisted by managementd.</p>
                </div>
                <div class="timeline">
                  <div v-for="event in overview?.recent_events ?? []" :key="event.id" class="timeline-item">
                    <strong>{{ event.action }}</strong>
                    <p>{{ event.summary }}</p>
                    <span>{{ new Date(event.occurred_at).toLocaleString() }}</span>
                  </div>
                </div>
              </article>
            </div>
          </section>

          <section v-if="view === 'devices'" class="device-layout">
            <article class="panel">
              <div class="panel-title">
                <h3>Device inventory</h3>
                <p>Search nodes, check online state, and drill into their peer view.</p>
              </div>

              <label class="field search-field">
                <span>Filter devices</span>
                <input v-model="search" type="search" placeholder="name, device id, overlay ip" />
              </label>

              <div class="device-list">
                <button
                  v-for="device in filteredDevices"
                  :key="device.id"
                  :class="['device-row', { selected: device.id === selectedDeviceId }]"
                  @click="loadDeviceDetail(device.id)"
                >
                  <div>
                    <strong>{{ device.name }}</strong>
                    <p>{{ device.id }} · {{ device.overlay_ip }}</p>
                  </div>
                  <div class="device-flags">
                    <span :class="['pill', device.online ? 'healthy' : 'neutral']">
                      {{ device.online ? 'online' : 'offline' }}
                    </span>
                    <span :class="['pill', device.disabled ? 'degraded' : 'healthy']">
                      {{ device.disabled ? 'disabled' : 'enabled' }}
                    </span>
                  </div>
                </button>
              </div>
            </article>

            <article class="panel detail-panel" v-if="selectedDevice && deviceDetail">
              <div class="panel-title">
                <h3>{{ selectedDevice.name }}</h3>
                <p>{{ selectedDevice.id }} · {{ selectedDevice.public_key }}</p>
              </div>

              <div class="detail-grid">
                <div class="detail-card">
                  <span>OS</span>
                  <strong>{{ selectedDevice.os }} {{ selectedDevice.version }}</strong>
                </div>
                <div class="detail-card">
                  <span>Last seen</span>
                  <strong>{{ new Date(selectedDevice.last_seen).toLocaleString() }}</strong>
                </div>
                <div class="detail-card">
                  <span>Direct endpoint</span>
                  <strong>
                    {{
                      selectedDevice.direct_endpoint
                        ? `${selectedDevice.direct_endpoint.host}:${selectedDevice.direct_endpoint.port}`
                        : 'not advertised'
                    }}
                  </strong>
                </div>
              </div>

              <form class="edit-form" @submit.prevent="saveDevice">
                <label class="field">
                  <span>Display name</span>
                  <input v-model="editState.name" type="text" />
                </label>
                <label class="field">
                  <span>Labels</span>
                  <textarea
                    v-model="editState.labelsText"
                    rows="5"
                    placeholder="site=shanghai&#10;role=edge"
                  ></textarea>
                </label>
                <label class="field">
                  <span>Advertised routes</span>
                  <textarea
                    v-model="editState.routesText"
                    rows="5"
                    placeholder="10.20.0.0/24&#10;10.30.0.0/24"
                  ></textarea>
                </label>

                <div class="action-row">
                  <button class="primary-button" :disabled="saving">
                    {{ saving ? 'Saving…' : 'Save changes' }}
                  </button>
                  <button
                    type="button"
                    class="ghost-button"
                    :disabled="saving"
                    @click="toggleDevice(!selectedDevice.disabled)"
                  >
                    {{ selectedDevice.disabled ? 'Enable device' : 'Disable device' }}
                  </button>
                  <button type="button" class="danger-button" :disabled="saving" @click="deleteDevice">
                    Delete device
                  </button>
                </div>
              </form>

              <div class="panel-title section-gap">
                <h3>Visible peers</h3>
                <p>The current peer view generated by managementd hooks.</p>
              </div>
              <div class="peer-list">
                <div v-for="peer in deviceDetail.peers" :key="peer.peer_id" class="peer-card">
                  <strong>{{ peer.peer_id }}</strong>
                  <p>{{ peer.overlay_ip }}</p>
                  <code>{{ peer.allowed_ips.join(', ') }}</code>
                </div>
              </div>
            </article>
          </section>

          <section v-if="view === 'services'" class="two-column">
            <article class="panel">
              <div class="panel-title">
                <h3>Service status</h3>
                <p>Raw internal status contracts aggregated by managementd.</p>
              </div>
              <div class="service-list">
                <div v-for="service in services" :key="service.service" class="service-card">
                  <div class="service-heading">
                    <strong>{{ service.service }}</strong>
                    <span :class="['pill', service.healthy ? 'healthy' : 'degraded']">
                      {{ service.healthy ? 'healthy' : 'degraded' }}
                    </span>
                  </div>
                  <pre class="service-status">{{ JSON.stringify(service.status ?? {}, null, 2) }}</pre>
                  <p v-if="service.error" class="service-error">{{ service.error }}</p>
                </div>
              </div>
            </article>

            <article class="panel">
              <div class="panel-title">
                <h3>Configuration summary</h3>
                <p>Read-only runtime configuration exposed by managementd.</p>
              </div>
              <div class="config-grid" v-if="config">
                <div class="detail-card">
                  <span>gRPC listen</span>
                  <strong>{{ config.grpc_listen_addr }}</strong>
                </div>
                <div class="detail-card">
                  <span>HTTP listen</span>
                  <strong>{{ config.http_listen_addr }}</strong>
                </div>
                <div class="detail-card">
                  <span>Overlay CIDR</span>
                  <strong>{{ config.overlay_cidr }}</strong>
                </div>
                <div class="detail-card">
                  <span>Sync interval</span>
                  <strong>{{ config.sync_interval }}</strong>
                </div>
                <div class="detail-card">
                  <span>SQLite path</span>
                  <strong>{{ config.state_db_path }}</strong>
                </div>
                <div class="detail-card">
                  <span>Admin token</span>
                  <strong>{{ config.admin_token_configured ? 'configured' : 'missing' }}</strong>
                </div>
                <div class="detail-card">
                  <span>signald status URL</span>
                  <strong>{{ config.signald_status_url }}</strong>
                </div>
                <div class="detail-card">
                  <span>relayd status URL</span>
                  <strong>{{ config.relayd_status_url }}</strong>
                </div>
              </div>
            </article>
          </section>

          <section v-if="view === 'audit'" class="panel">
            <div class="panel-title">
              <h3>Audit ledger</h3>
              <p>Persistent device management and registration events.</p>
            </div>
            <div class="timeline">
              <div v-for="event in audit" :key="event.id" class="timeline-item">
                <div class="timeline-heading">
                  <strong>{{ event.action }}</strong>
                  <span>{{ new Date(event.occurred_at).toLocaleString() }}</span>
                </div>
                <p>{{ event.summary }}</p>
                <small>{{ event.actor }} · {{ event.device_id || 'n/a' }}</small>
              </div>
            </div>
          </section>
        </main>
      </div>
    </template>
  </div>
</template>

<style>
:root {
  color-scheme: dark;
  --bg: #08111d;
  --bg-alt: #0e1827;
  --panel: rgba(13, 26, 41, 0.78);
  --panel-strong: rgba(18, 33, 51, 0.96);
  --line: rgba(122, 167, 214, 0.16);
  --text: #eef6ff;
  --muted: #8ea5bf;
  --gold: #ffd47c;
  --mint: #5ad8b8;
  --rose: #ff8c82;
  --blue: #75b7ff;
  --shadow: 0 32px 80px rgba(0, 0, 0, 0.42);
  font-family: "IBM Plex Sans", "Segoe UI", sans-serif;
  background:
    radial-gradient(circle at top left, rgba(117, 183, 255, 0.18), transparent 26rem),
    radial-gradient(circle at bottom right, rgba(255, 212, 124, 0.12), transparent 24rem),
    var(--bg);
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  min-height: 100vh;
  color: var(--text);
  background: var(--bg);
}

button,
input,
textarea {
  font: inherit;
}

.shell {
  min-height: 100vh;
  position: relative;
  overflow: hidden;
}

.ambient {
  position: absolute;
  inset: auto;
  width: 32rem;
  height: 32rem;
  border-radius: 999px;
  filter: blur(80px);
  opacity: 0.38;
  pointer-events: none;
}

.ambient-left {
  top: -6rem;
  left: -8rem;
  background: rgba(117, 183, 255, 0.25);
}

.ambient-right {
  right: -10rem;
  bottom: -6rem;
  background: rgba(255, 212, 124, 0.18);
}

.frame,
.login-panel {
  position: relative;
  z-index: 1;
}

.login-panel {
  min-height: 100vh;
  display: grid;
  grid-template-columns: minmax(0, 1.3fr) minmax(18rem, 26rem);
  gap: 2rem;
  align-items: center;
  padding: 3rem;
}

.login-copy h1,
.masthead h2,
.sidebar h1 {
  font-family: "Iowan Old Style", "Palatino Linotype", serif;
  letter-spacing: 0.02em;
}

.login-copy h1 {
  margin: 0.5rem 0 1rem;
  font-size: clamp(2.8rem, 5vw, 5.6rem);
  line-height: 0.94;
  max-width: 12ch;
}

.lede,
.sidebar-copy,
.panel-title p,
.device-row p,
.timeline-item p,
.timestamp,
.service-error,
.timeline-item small {
  color: var(--muted);
}

.login-card,
.panel,
.sidebar,
.detail-card,
.stat-card,
.device-row,
.service-card,
.peer-card {
  border: 1px solid var(--line);
  background: var(--panel);
  backdrop-filter: blur(18px);
  box-shadow: var(--shadow);
}

.login-card {
  border-radius: 1.75rem;
  padding: 1.5rem;
}

.eyebrow {
  margin: 0;
  text-transform: uppercase;
  letter-spacing: 0.18em;
  font-size: 0.75rem;
  color: var(--gold);
}

.field {
  display: grid;
  gap: 0.5rem;
  margin-bottom: 1rem;
}

.field span {
  color: var(--muted);
}

.field input,
.field textarea {
  width: 100%;
  border-radius: 1rem;
  border: 1px solid rgba(255, 255, 255, 0.08);
  background: rgba(4, 10, 18, 0.76);
  color: var(--text);
  padding: 0.9rem 1rem;
}

.primary-button,
.ghost-button,
.danger-button,
.nav-item {
  border: 0;
  border-radius: 999px;
  padding: 0.9rem 1.15rem;
  cursor: pointer;
  transition: transform 160ms ease, opacity 160ms ease, background 160ms ease;
}

.primary-button:hover,
.ghost-button:hover,
.danger-button:hover,
.nav-item:hover {
  transform: translateY(-1px);
}

.primary-button {
  background: linear-gradient(135deg, var(--gold), #ffb86a);
  color: #201100;
  font-weight: 700;
}

.ghost-button {
  background: rgba(255, 255, 255, 0.04);
  color: var(--text);
}

.danger-button {
  background: rgba(255, 140, 130, 0.18);
  color: #ffd4d1;
}

.frame {
  display: grid;
  grid-template-columns: 18rem minmax(0, 1fr);
  gap: 1.5rem;
  min-height: 100vh;
  padding: 1.5rem;
}

.sidebar {
  border-radius: 2rem;
  padding: 1.5rem;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}

.sidebar h1 {
  margin: 0.35rem 0 0.85rem;
  font-size: 2rem;
}

.nav {
  display: grid;
  gap: 0.65rem;
  margin: 2rem 0;
}

.nav-item {
  text-align: left;
  background: transparent;
  color: var(--muted);
  border: 1px solid transparent;
}

.nav-item.active {
  background: rgba(117, 183, 255, 0.12);
  color: var(--text);
  border-color: rgba(117, 183, 255, 0.24);
}

.sidebar-actions {
  display: grid;
  gap: 0.75rem;
}

.content {
  display: grid;
  gap: 1.25rem;
  align-content: start;
}

.masthead {
  display: flex;
  justify-content: space-between;
  align-items: end;
  gap: 1rem;
}

.masthead h2 {
  margin: 0.35rem 0 0;
  font-size: clamp(2rem, 3vw, 3.4rem);
}

.panel-stack,
.two-column,
.device-layout {
  display: grid;
  gap: 1rem;
}

.stat-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 1rem;
}

.stat-card,
.detail-card {
  border-radius: 1.5rem;
  padding: 1rem;
}

.stat-card span,
.detail-card span {
  display: block;
  color: var(--muted);
  margin-bottom: 0.35rem;
}

.stat-card strong,
.detail-card strong {
  font-size: 1.25rem;
}

.two-column {
  grid-template-columns: minmax(0, 1.35fr) minmax(20rem, 1fr);
}

.device-layout {
  grid-template-columns: minmax(18rem, 0.9fr) minmax(0, 1.4fr);
}

.panel {
  border-radius: 2rem;
  padding: 1.2rem;
}

.panel-title h3 {
  margin: 0;
  font-size: 1.2rem;
}

.panel-title p {
  margin: 0.35rem 0 0;
}

.search-field {
  margin-top: 1rem;
}

.device-list,
.service-list,
.peer-list,
.timeline {
  display: grid;
  gap: 0.75rem;
  margin-top: 1rem;
}

.device-row,
.service-card,
.peer-card {
  border-radius: 1.4rem;
  padding: 1rem;
}

.device-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  text-align: left;
}

.device-row.selected {
  border-color: rgba(255, 212, 124, 0.28);
  background: rgba(255, 212, 124, 0.08);
}

.device-flags {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
  justify-content: end;
}

.pill {
  display: inline-flex;
  align-items: center;
  padding: 0.35rem 0.75rem;
  border-radius: 999px;
  font-size: 0.8rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}

.pill.healthy {
  background: rgba(90, 216, 184, 0.12);
  color: var(--mint);
}

.pill.degraded {
  background: rgba(255, 140, 130, 0.14);
  color: var(--rose);
}

.pill.neutral {
  background: rgba(117, 183, 255, 0.12);
  color: var(--blue);
}

.detail-panel {
  align-content: start;
}

.detail-grid,
.config-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.75rem;
  margin-top: 1rem;
}

.edit-form {
  margin-top: 1rem;
}

.action-row {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
}

.section-gap {
  margin-top: 1.5rem;
}

.peer-card code,
.service-status {
  display: block;
  margin-top: 0.5rem;
  font-family: "IBM Plex Mono", "SFMono-Regular", monospace;
  font-size: 0.85rem;
  white-space: pre-wrap;
  word-break: break-word;
}

.service-status {
  background: rgba(4, 10, 18, 0.66);
  padding: 1rem;
  border-radius: 1rem;
}

.timeline-item {
  border-left: 2px solid rgba(255, 212, 124, 0.24);
  padding-left: 1rem;
}

.timeline-heading {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
}

.error-banner {
  margin: 0;
  padding: 0.85rem 1rem;
  border-radius: 1rem;
  background: rgba(255, 140, 130, 0.12);
  border: 1px solid rgba(255, 140, 130, 0.25);
  color: #ffe0dd;
}

@media (max-width: 1100px) {
  .frame,
  .device-layout,
  .two-column,
  .login-panel,
  .stat-grid {
    grid-template-columns: 1fr;
  }

  .frame {
    padding: 1rem;
  }

  .sidebar {
    gap: 1.5rem;
  }

  .detail-grid,
  .config-grid {
    grid-template-columns: 1fr;
  }

  .masthead {
    flex-direction: column;
    align-items: start;
  }
}
</style>
