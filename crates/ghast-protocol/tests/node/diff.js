'use strict'
// Differential test helper: node-minecraft-protocol as the reference implementation.
//
// Usage: node diff.js <requests.jsonl>
// Each request line is one of:
//   {"op":"reparse","version":"1.8.9","state":"play","dir":"toClient","hex":"..."}
//     -> parse the packet (id + body) with node, check that every byte was consumed,
//        serialize the parsed value again and return its hex.
//   {"op":"parse","version":"1.8.9","state":"play","dir":"toClient","hex":"..."}
//     -> parse with node and return the packet name and its parsed fields as JSON
//        (BigInt as string, Buffer as hex, 64-bit pairs as [hi, lo]).
//   {"op":"gen","version":"1.8.9","state":"play","dir":"toClient","name":"map_chunk","count":N,"seed":S}
//     -> generate N random packets from the protodef schema (independently of the Rust
//        generator), serialize them with node and return their hex.
// One JSON response line is printed per request (gen: one line with a list).

const fs = require('fs')
const mc = require('minecraft-protocol/src/transforms/serializer')
const minecraftData = require('minecraft-data')
const { ProtoDefCompiler } = require('protodef').Compiler
const nbt = require('prismarine-nbt')

// --- protodef instances (compiled, like minecraft-protocol uses them) ---------------
const protos = {}
function proto (version, state, dir) {
  const key = `${version};${state};${dir}`
  if (!protos[key]) {
    const data = minecraftData(version)
    const compiler = new ProtoDefCompiler()
    compiler.addTypes(require('minecraft-protocol/src/datatypes/compiler-minecraft'))
    compiler.addProtocol(data.protocol, [state, dir])
    nbt.addTypesToCompiler('big', compiler)
    protos[key] = compiler.compileProtoDefSync()
  }
  return protos[key]
}
void mc

// --- deterministic RNG -----------------------------------------------------------------
let rngState = 1n
function seed (s) { rngState = BigInt.asUintN(64, BigInt(s) ^ 0x9e3779b97f4a7c15n) }
function next64 () {
  rngState = BigInt.asUintN(64, rngState + 0x9e3779b97f4a7c15n)
  let z = rngState
  z = BigInt.asUintN(64, (z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n)
  z = BigInt.asUintN(64, (z ^ (z >> 27n)) * 0x94d049bb133111ebn)
  return z ^ (z >> 31n)
}
function below (n) { return Number(next64() % BigInt(n)) }
function chance (p) { return below(100) < p }
function pick (a) { return a[below(a.length)] }
function intIn (lo, hi) {
  switch (below(6)) {
    case 0: return Math.max(lo, Math.min(hi, below(20) - 3))
    case 1: return lo
    case 2: return hi
    default: return lo + Number(next64() % BigInt(hi - lo + 1))
  }
}
function text () {
  const chars = ['a', 'b', 'Z', '0', ' ', ':', '"', '§', 'é', '日', '😀']
  let s = ''
  const n = below(10)
  for (let i = 0; i < n; i++) s += pick(chars)
  return s
}
function bmpText () {
  const chars = ['a', 'b', 'Z', '0', ' ', ':', 'é', '日']
  let s = ''
  const n = below(8)
  for (let i = 0; i < n; i++) s += pick(chars)
  return s
}
function len () { return chance(90) ? below(4) : below(13) }
function f32 () {
  if (chance(25)) return pick([0, 1, -1.5, 0.25, 64])
  const b = Buffer.alloc(4)
  for (;;) {
    b.writeUInt32BE(Number(next64() & 0xffffffffn))
    const v = b.readFloatBE(0)
    if (Number.isFinite(v)) return v
  }
}
function f64 () {
  if (chance(25)) return pick([0, 1, -1.5, 0.25, 1e300])
  const b = Buffer.alloc(8)
  for (;;) {
    b.writeBigUInt64BE(next64())
    const v = b.readDoubleBE(0)
    if (Number.isFinite(v)) return v
  }
}
function i32 () { return intIn(-2147483648, 2147483647) }
function i64 () { return [i32(), i32()] }
function uuid () {
  const h = []
  for (let i = 0; i < 16; i++) h.push(below(256).toString(16).padStart(2, '0'))
  const s = h.join('')
  return `${s.slice(0, 8)}-${s.slice(8, 12)}-${s.slice(12, 16)}-${s.slice(16, 20)}-${s.slice(20)}`
}
function bytes () {
  const n = below(25)
  const b = Buffer.alloc(n)
  for (let i = 0; i < n; i++) b[i] = below(256)
  return b
}

// --- NBT in prismarine-nbt's value format ----------------------------------------------
const NBT_TYPES = ['byte', 'short', 'int', 'long', 'float', 'double', 'byteArray', 'string', 'list', 'compound', 'intArray', 'longArray']
function nbtValue (type, depth) {
  switch (type) {
    case 'byte': return intIn(-128, 127)
    case 'short': return intIn(-32768, 32767)
    case 'int': return i32()
    case 'long': return i64()
    case 'float': return f32()
    case 'double': return f64()
    case 'byteArray': { const n = len(); const a = []; for (let i = 0; i < n; i++) a.push(intIn(-128, 127)); return a }
    case 'string': return bmpText()
    case 'intArray': { const n = len(); const a = []; for (let i = 0; i < n; i++) a.push(i32()); return a }
    case 'longArray': { const n = len(); const a = []; for (let i = 0; i < n; i++) a.push(i64()); return a }
    case 'list': {
      const et = depth >= 3 ? pick(NBT_TYPES.slice(0, 8)) : pick(NBT_TYPES)
      const n = len()
      const a = []
      for (let i = 0; i < n; i++) a.push(nbtValue(et, depth + 1))
      return { type: n === 0 && chance(50) ? 'end' : et, value: a }
    }
    case 'compound': {
      const o = {}
      const n = depth >= 3 ? 0 : len()
      for (let i = 0; i < n; i++) {
        const t = pick(NBT_TYPES)
        o[`k${i}${bmpText()}`] = { type: t, value: nbtValue(t, depth + 1) }
      }
      return o
    }
  }
  throw new Error('nbt type ' + type)
}
function nbtRoot () {
  return { type: 'compound', name: chance(70) ? '' : bmpText(), value: nbtValue('compound', 0) }
}

// --- schema-driven packet generator ----------------------------------------------------
// Values that switches compare against, by field name (scope-insensitive bias only),
// and field names used as array counts.
function scanHints (protocol) {
  const hints = {}
  const counts = new Set()
  function walk (t) {
    if (!Array.isArray(t)) return
    const [k, a] = t
    if (k === 'container') a.forEach(f => walk(f.type))
    else if (k === 'switch') {
      const name = a.compareTo.split('/').filter(s => s !== '..').pop()
      hints[name] = (hints[name] || []).concat(Object.keys(a.fields || {}))
      Object.values(a.fields || {}).forEach(walk)
      if (a.default) walk(a.default)
    } else if (k === 'array') {
      if (typeof a.count === 'string') counts.add(a.count)
      walk(a.type)
    } else if (k === 'option') walk(a)
    else if (k === 'entityMetadataLoop' || k === 'topBitSetTerminatedArray') walk(a.type)
  }
  function all (o) { for (const v of Object.values(o)) walk(v) }
  all(protocol.types)
  for (const st of ['handshaking', 'status', 'login', 'play']) {
    for (const d of ['toClient', 'toServer']) all(protocol[st][d].types)
  }
  return { hints, counts }
}

function makeGen (protocol, state, dir) {
  const types = Object.assign({}, protocol.types, protocol[state][dir].types)
  const { hints, counts } = scanHints(protocol)

  function lookup (path, stack) {
    const segs = path.split('/')
    let lvl = stack.length - 1
    let v
    for (const s of segs) {
      if (s === '..') { lvl--; continue }
      v = v === undefined ? stack[lvl][s] : v[s]
    }
    return v
  }

  function intField (name, lo, hi) {
    if (counts.has(name)) return below(4)
    const h = (hints[name] || []).map(Number).filter(n => Number.isInteger(n) && n >= lo && n <= hi)
    if (h.length && chance(65)) return pick(h)
    return intIn(lo, hi)
  }

  function gen (t, name, stack, compareTo) {
    if (typeof t === 'string') {
      switch (t) {
        case 'varint': case 'optvarint': case 'i32': return intField(name, -2147483648, 2147483647)
        case 'varlong': return intField(name, -2147483648, 2147483647) // node's varlong is 32-bit
        case 'i8': return intField(name, -128, 127)
        case 'u8': return intField(name, 0, 255)
        case 'i16': return intField(name, -32768, 32767)
        case 'u16': return intField(name, 0, 65535)
        case 'i64': return i64()
        case 'f32': return f32()
        case 'f64': return f64()
        case 'bool': return chance(50)
        case 'UUID': return uuid()
        case 'string': {
          const h = hints[name] || []
          if (h.length && chance(80)) return pick(h)
          return text()
        }
        case 'ByteArray': case 'restBuffer': return bytes()
        case 'nbt': return nbtRoot()
        case 'optionalNbt': return chance(30) ? undefined : nbtRoot()
        case 'void': return undefined
      }
      if (types[t] === undefined) throw new Error('unknown type ' + t)
      return gen(types[t], name, stack, compareTo)
    }
    const [k, a] = t
    switch (k) {
      case 'container': {
        const obj = {}
        stack.push(obj)
        for (const f of a) {
          const v = gen(f.type, f.name, stack, compareTo)
          if (f.anon) { if (v && typeof v === 'object') Object.assign(obj, v) } else if (v !== undefined) obj[f.name] = v
        }
        stack.pop()
        return obj
      }
      case 'switch': {
        const ct = a.compareTo === '$compareTo' ? compareTo : a.compareTo
        const d = lookup(ct, stack)
        const key = typeof d === 'boolean' ? String(d) : String(d)
        const branch = (a.fields && Object.prototype.hasOwnProperty.call(a.fields, key)) ? a.fields[key] : (a.default || 'void')
        return gen(branch, name, stack, compareTo)
      }
      case 'array': {
        let n
        if (a.countType) n = len()
        else if (typeof a.count === 'number') n = a.count
        else n = lookup(a.count, stack)
        const out = []
        for (let i = 0; i < n; i++) out.push(gen(a.type, name, stack, compareTo))
        return out
      }
      case 'option': return chance(50) ? gen(a, name, stack, compareTo) : undefined
      case 'bitfield': {
        const o = {}
        for (const f of a) {
          const lo = f.signed ? -(2 ** (f.size - 1)) : 0
          const hi = f.signed ? 2 ** (f.size - 1) - 1 : 2 ** f.size - 1
          const h = (hints[f.name] || []).map(Number).filter(n => Number.isInteger(n) && n >= lo && n <= hi)
          o[f.name] = h.length && chance(65) ? pick(h) : intIn(lo, hi)
        }
        return o
      }
      case 'mapper': return pick(Object.values(a.mappings))
      case 'buffer': return bytes()
      case 'pstring': return text()
      case 'entityMetadataLoop': {
        const n = len()
        const out = []
        while (out.length < n) {
          const e = gen(a.type, name, stack, compareTo)
          out.push(e)
        }
        return out
      }
      case 'topBitSetTerminatedArray': {
        // The first byte of each entry carries the "more follows" flag, so the first
        // field must fit in 7 bits.
        let et = a.type
        while (typeof et === 'string') et = types[et]
        const first = et[1][0].name
        const n = 1 + len()
        const out = []
        for (let i = 0; i < n; i++) {
          const e = gen(a.type, name, stack, compareTo)
          e[first] = e[first] & 127
          out.push(e)
        }
        return out
      }
      default: {
        if (types[k] !== undefined) {
          // Parametrised named type, e.g. ["particleData", {"compareTo": "particleId"}].
          return gen(types[k], name, stack, a.compareTo)
        }
        throw new Error('unsupported type ' + k)
      }
    }
  }
  return (packetName) => gen(types['packet_' + packetName], packetName, [], undefined)
}

// --- main -----------------------------------------------------------------------------
const gens = {}
const lines = fs.readFileSync(process.argv[2], 'utf8').split('\n').filter(Boolean)
const out = []
for (const line of lines) {
  const req = JSON.parse(line)
  try {
    const p = proto(req.version, req.state, req.dir)
    if (req.op === 'reparse') {
      const buf = Buffer.from(req.hex, 'hex')
      const parsed = p.parsePacketBuffer('packet', buf)
      if (parsed.metadata.size !== buf.length) {
        out.push({ ok: false, err: `consumed ${parsed.metadata.size} of ${buf.length} bytes` })
        continue
      }
      const re = p.createPacketBuffer('packet', parsed.data)
      out.push({ ok: true, name: parsed.data.name, hex: re.toString('hex') })
    } else if (req.op === 'parse') {
      const buf = Buffer.from(req.hex, 'hex')
      const parsed = p.parsePacketBuffer('packet', buf)
      const json = JSON.parse(JSON.stringify(parsed.data.params, (k, v) => {
        if (typeof v === 'bigint') return v.toString()
        if (v && v.type === 'Buffer' && Array.isArray(v.data)) return Buffer.from(v.data).toString('hex')
        return v
      }))
      out.push({ ok: true, name: parsed.data.name, size: parsed.metadata.size, params: json })
    } else if (req.op === 'gen') {
      const key = `${req.version};${req.state};${req.dir}`
      if (!gens[key]) gens[key] = makeGen(minecraftData(req.version).protocol, req.state, req.dir)
      seed(req.seed)
      const hexes = []
      for (let i = 0; i < req.count; i++) {
        let params
        for (let attempt = 0; ; attempt++) {
          params = gens[key](req.name)
          // ghast follows vanilla where minecraft-data does not (see diverges_from_node in
          // differential.rs): 1.8 spawn_entity has velocity iff objectData > 0.
          if (req.version.startsWith('1.8') && req.name === 'spawn_entity' && params.objectData < 0) {
            params.objectData = -(params.objectData + 1)
            if (params.objectData === 0) delete params.velocity
          }
          // Skip values whose encoding would contain an entity-metadata terminator or
          // other node-specific failure; retried with fresh randomness.
          try {
            const b = p.createPacketBuffer('packet', { name: req.name, params })
            // Hex first: node's topBitSetTerminatedArray reader clears flag bits in the
            // buffer it parses.
            const hx = b.toString('hex')
            const back = p.parsePacketBuffer('packet', b)
            if (back.metadata.size !== b.length) throw new Error('size')
            hexes.push(hx)
            break
          } catch (e) {
            if (attempt > 50) throw e
          }
        }
      }
      out.push({ ok: true, hexes })
    } else {
      out.push({ ok: false, err: 'unknown op' })
    }
  } catch (e) {
    out.push({ ok: false, err: String(e && e.message ? e.message : e).slice(0, 300) })
  }
}
process.stdout.write(out.map(o => JSON.stringify(o)).join('\n') + '\n')
