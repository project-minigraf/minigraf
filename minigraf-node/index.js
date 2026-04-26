'use strict'

const { platform, arch } = process

let nativeBinding = null
const loadErrors = []

if (platform === 'linux') {
  if (arch === 'x64') {
    try { nativeBinding = require('@minigraf/linux-x64-gnu') }
    catch (e) { loadErrors.push(e) }
  } else if (arch === 'arm64') {
    try { nativeBinding = require('@minigraf/linux-arm64-gnu') }
    catch (e) { loadErrors.push(e) }
  }
} else if (platform === 'darwin') {
  try { nativeBinding = require('@minigraf/darwin-universal') }
  catch (e) { loadErrors.push(e) }
} else if (platform === 'win32') {
  if (arch === 'x64') {
    try { nativeBinding = require('@minigraf/win32-x64-msvc') }
    catch (e) { loadErrors.push(e) }
  }
}

if (!nativeBinding) {
  const errorMessages = loadErrors.map(e => e.message).join('\n')
  throw new Error(
    `Failed to load native Minigraf addon for ${platform}-${arch}.\n` +
    `This platform may not be supported yet.\n` +
    (errorMessages ? `Errors:\n${errorMessages}` : '')
  )
}

module.exports = nativeBinding
