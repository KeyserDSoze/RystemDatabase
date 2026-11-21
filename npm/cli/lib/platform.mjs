export function platformAsset(platform = process.platform, arch = process.arch) {
  const key = platform + '-' + arch;
  const assets = {
    'linux-x64': 'rystemdb-linux-x64',
    'linux-arm64': 'rystemdb-linux-arm64',
    'darwin-x64': 'rystemdb-darwin-x64',
    'darwin-arm64': 'rystemdb-darwin-arm64',
    'win32-x64': 'rystemdb-windows-x64.exe'
  };
  const asset = assets[key];
  if (!asset) throw new Error('Unsupported platform: ' + key);
  return asset;
}
