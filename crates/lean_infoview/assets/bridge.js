// Connects the Lean infoview to Zed. Messages to Zed go through a WKScriptMessageHandler,
// and Zed answers by evaluating `window.zedInfoview.*` calls.
import { loadRenderInfoview } from './vendor/loader.production.min.js'

const host = window.webkit.messageHandlers.zed
const pending = new Map()
let nextCallId = 0
let infoview

function post(message) {
    host.postMessage(JSON.stringify(message))
}

function start(method, params) {
    const id = ++nextCallId
    const promise = new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        post({ kind: 'call', id, call: { method, params } })
    })
    return { id, promise }
}

function invoke(method, params) {
    return start(method, params).promise
}

for (const level of ['error', 'warn']) {
    const original = console[level].bind(console)
    console[level] = (...args) => {
        original(...args)
        post({ kind: 'log', level, message: args.map(String).join(' ') })
    }
}
window.addEventListener('error', event => {
    post({ kind: 'log', level: 'error', message: `${event.message} (${event.filename}:${event.lineno})` })
})
window.addEventListener('unhandledrejection', event => {
    const reason = event.reason
    post({ kind: 'log', level: 'error', message: String(reason?.stack ?? reason?.message ?? reason) })
})

const editorApi = {
    saveConfig: config => invoke('saveConfig', { config }),
    sendClientRequest(uri, method, params, options) {
        const { id, promise } = start('sendClientRequest', { uri, method, params })
        options?.abortSignal?.addEventListener('abort', () => post({ kind: 'cancel', id }))
        return promise
    },
    sendClientNotification: (uri, method, params) => invoke('sendClientNotification', { uri, method, params }),
    subscribeServerNotifications: method => invoke('subscribe', { direction: 'fromServer', method }),
    unsubscribeServerNotifications: method => invoke('unsubscribe', { direction: 'fromServer', method }),
    subscribeClientNotifications: method => invoke('subscribe', { direction: 'fromClient', method }),
    unsubscribeClientNotifications: method => invoke('unsubscribe', { direction: 'fromClient', method }),
    copyToClipboard: text => invoke('copyToClipboard', { text }),
    insertText: (text, kind, pos) => invoke('insertText', { text, kind, pos }),
    applyEdit: edit => invoke('applyEdit', { edit }),
    showDocument: show => invoke('showDocument', { show }),
    restartFile: uri => invoke('restartFile', { uri }),
    createRpcSession: uri => invoke('createRpcSession', { uri }),
    closeRpcSession: sessionId => invoke('closeRpcSession', { sessionId }),
    readPersistentState: async (container, key) => (await invoke('readState', { container, key })) ?? undefined,
    savePersistentState: (container, key, value) => invoke('saveState', { container, key, value }),
    deletePersistentState: (container, key) => invoke('deleteState', { container, key }),
}

window.zedInfoview = {
    reply(id, outcome) {
        const call = pending.get(id)
        if (call === undefined) return
        pending.delete(id)
        if ('error' in outcome) call.reject(outcome.error)
        else call.resolve(outcome.ok)
    },
    call(method, args) {
        infoview[method](...args).catch(error => console.error(`infoview.${method} failed: ${error}`))
    },
    setAppearance(appearance) {
        document.body.classList.remove('vscode-light', 'vscode-dark')
        document.body.classList.add(`vscode-${appearance}`)
        document.getElementById('zed-theme').href = `theme.css?${Date.now()}`
    },
}

const vendor = file => new URL(`vendor/${file}`, import.meta.url).href
loadRenderInfoview(
    {
        '@leanprover/infoview': vendor('index.production.min.js'),
        react: vendor('react.production.min.js'),
        'react/jsx-runtime': vendor('react-jsx-runtime.production.min.js'),
        'react-dom': vendor('react-dom.production.min.js'),
    },
    [editorApi, document.getElementById('react_root')],
    api => {
        infoview = api
        post({ kind: 'ready' })
    },
)
