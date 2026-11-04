// Grim positive control: join, then send movement Grim must flag (rise 0.5 block/tick with onGround=true,
// no allowFlight), so a lab can prove Grim checks and prints flags for that protocol version.
//   node grim-control.mjs [--port 25618] [--version 1.8.9] [--user Baseline2] [--ticks 40]
import mineflayer from 'mineflayer'
const arg = (k, d) => { const i = process.argv.indexOf('--' + k); return i > 0 ? process.argv[i + 1] : d }
const port = Number(arg('port', '25618'))
const version = arg('version', port === 25616 ? '1.16.5' : '1.8.9')
const ticks = Number(arg('ticks', '40'))
const bot = mineflayer.createBot({ host: arg('host', '127.0.0.1'), port, version, username: arg('user', 'Baseline2'), auth: 'offline' })
const log = (...a) => console.log(new Date().toISOString().slice(11, 23), ...a)
bot.on('kicked', (r) => log('kicked', JSON.stringify(r)))
bot.on('end', (r) => { log('end', r); process.exit(0) })
bot.once('spawn', () => {
  log('spawned at', bot.entity.position.toString())
  setTimeout(() => {
    bot.physicsEnabled = false
    const p = bot.entity.position.clone()
    let n = 0
    log(`sending ${ticks} illegal moves (y += 0.5 per tick, onGround=true)`)
    const iv = setInterval(() => {
      p.y += 0.5
      bot._client.write('position', { x: p.x, y: p.y, z: p.z, onGround: true })
      if (++n >= ticks) { clearInterval(iv); log('done; staying 3 s for setbacks'); setTimeout(() => bot.quit(), 3000) }
    }, 50)
  }, 3000)
})
