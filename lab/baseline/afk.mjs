// mineflayer AFK baseline: join, spawn, stand still (default physics on, as a real bot would), log once a minute.
//   node afk.mjs [--host 127.0.0.1] [--port 25618] [--version 1.8.9] [--user Baseline] [--secs 0 = forever]
import mineflayer from 'mineflayer'

const arg = (k, d) => { const i = process.argv.indexOf('--' + k); return i > 0 ? process.argv[i + 1] : d }
const host = arg('host', '127.0.0.1')
const port = Number(arg('port', '25618'))
const version = arg('version', port === 25616 ? '1.16.5' : '1.8.9')
const username = arg('user', 'Baseline')
const secs = Number(arg('secs', '0'))
const t0 = Date.now()
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a)

const opts = { host, port, version, username, auth: 'offline', checkTimeoutInterval: 30000, hideErrors: false }
log(`pid ${process.pid} connecting ${username}@${host}:${port} (${version})`)
const bot = mineflayer.createBot(opts)
let packets = 0
bot._client.on('packet', () => { packets++ })
bot.once('spawn', () => log(`spawned at ${bot.entity.position} after ${Date.now() - t0} ms`))
bot.on('kicked', (r) => log('kicked', typeof r === 'string' ? r : JSON.stringify(r)))
bot.on('error', (e) => log('error', e.message))
bot.on('end', (r) => { log('end', r); process.exit(1) })
setInterval(() => log(`alive ${Math.round((Date.now() - t0) / 1000)} s, ${packets} packets in, rss ${(process.memoryUsage().rss / 1048576).toFixed(1)} MB`), 60000)
if (secs > 0) setTimeout(() => { log('done'); bot.quit(); setTimeout(() => process.exit(0), 500) }, secs * 1000)
