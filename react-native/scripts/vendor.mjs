// what the podspec reads outside the repository, copied into the package: the Swift package's sources and the XCFramework's zip (ios/build.sh --zip)
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const root = join(import.meta.dirname, '..');
const repo = join(root, '..');
const vendor = join(root, 'ios', 'vendor');
const zip = join(repo, 'ios', 'build', 'AlkoveFFI.xcframework.zip');
if (!existsSync(zip)) throw new Error(`${zip} is missing: run ios/build.sh --zip first`);

rmSync(vendor, { recursive: true, force: true });
mkdirSync(join(vendor, 'Alkove'), { recursive: true });
const sources = join(repo, 'ios', 'Sources', 'Alkove');
for (const f of readdirSync(sources).filter((f) => f.endsWith('.swift'))) copyFileSync(join(sources, f), join(vendor, 'Alkove', f));
copyFileSync(zip, join(vendor, 'AlkoveFFI.xcframework.zip'));
console.log(`ios/vendor: ${readdirSync(join(vendor, 'Alkove')).length} Swift files and the XCFramework's zip`);
