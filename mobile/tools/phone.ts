// Renews an expired free-team iOS provisioning profile if needed, builds and
// installs the Exact app onto the connected iPhone, attempts to launch it, and
// prints the iOS trust steps when done.
//   bun tools/phone.ts [--refresh] [--phone <udid|name>]
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

interface Device {
  name: string;
  udid: string;
  reachable: boolean;
}

interface Profile {
  path: string;
  team?: string;
  appId?: string;
  prefix?: string;
  devices: string[];
  platforms: string[];
  dev: boolean;
  expires: Date;
  developer?: string;
}

const rawArgs = process.argv.slice(2);
const forceRefresh = rawArgs.includes('--refresh');
const forwardArgs = rawArgs.filter((arg) => arg !== '--refresh');

const phoneFlagIndex = forwardArgs.indexOf('--phone');
const phonePick = phoneFlagIndex >= 0 ? forwardArgs[phoneFlagIndex + 1] : process.env.EXACT_PHONE;

const appJson = JSON.parse(readFileSync(resolve(import.meta.dir, '..', 'app.json'), 'utf8'));
const bundleId: string = appJson.app?.id ?? appJson.id;
const appName: string = appJson.app?.name ?? appJson.name ?? 'Credits';
const team = process.env.DEVELOPMENT_TEAM?.trim();

if (!bundleId) {
  console.error('mobile/app.json is missing app.id');
  process.exit(1);
}

function findPhone(pick?: string): Device {
  const dir = mkdtempSync(join(tmpdir(), 'credits-devices-'));
  const out = join(dir, 'devices.json');
  try {
    const res = spawnSync('xcrun', ['devicectl', 'list', 'devices', '--json-output', out], { encoding: 'utf8' });
    if (res.status !== 0 || !existsSync(out)) {
      throw new Error(res.stderr?.trim() || 'xcrun devicectl list devices failed');
    }
    const parsed = JSON.parse(readFileSync(out, 'utf8'));
    const devices: Device[] = (parsed.result?.devices ?? [])
      .filter((d: any) => d.hardwareProperties?.reality === 'physical' && d.hardwareProperties?.platform === 'iOS')
      .map((d: any) => ({
        name: d.deviceProperties?.name ?? d.hardwareProperties?.udid,
        udid: d.hardwareProperties?.udid,
        reachable: d.connectionProperties?.tunnelState === 'connected',
      }))
      .filter((d: Device) => Boolean(d.udid));

    const matched = pick
      ? devices.find((d) => d.udid === pick || d.name === pick)
      : devices.find((d) => d.reachable) ?? (devices.length === 1 ? devices[0] : undefined);

    if (!matched) {
      throw new Error(
        pick
          ? `No physical iPhone matching "${pick}" found (xcrun devicectl list devices).`
          : 'No iPhone found: plug your iPhone in over USB, unlock it, and trust this Mac.',
      );
    }
    if (!matched.reachable) {
      throw new Error(`${matched.name} (${matched.udid}) is not connected: plug it in over USB and unlock it.`);
    }
    return matched;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function extractDeveloperName(certBase64?: string): string | undefined {
  if (!certBase64) return undefined;
  const der = Buffer.from(certBase64.replace(/\s+/g, ''), 'base64');
  const res = spawnSync('openssl', ['x509', '-inform', 'DER', '-noout', '-subject'], { input: der });
  if (res.status !== 0) return undefined;
  const subject = res.stdout.toString('utf8');
  const cn = /CN\s*=\s*([^,/\n]+)/.exec(subject)?.[1]?.trim();
  if (!cn) return undefined;
  // iOS VPN & Device Management displays "Apple Development: email" without the trailing "(TEAMID)".
  return cn.replace(/\s*\([A-Z0-9]{10}\)$/, '');
}

function decodeProfile(path: string): Profile | null {
  const res = spawnSync('security', ['cms', '-D', '-i', path], { encoding: 'utf8' });
  if (res.status !== 0) return null;
  const xml = res.stdout ?? '';
  const str = (key: string) => new RegExp(`<key>${key}</key>\\s*<string>([^<]*)</string>`).exec(xml)?.[1];
  const strings = (key: string) =>
    [...(new RegExp(`<key>${key}</key>\\s*<array>([\\s\\S]*?)</array>`).exec(xml)?.[1] ?? '').matchAll(/<string>([^<]*)<\/string>/g)].map(
      (m) => m[1],
    );
  const appId = str('application-identifier') ?? str('com.apple.application-identifier');
  const teamId = /<key>TeamIdentifier<\/key>\s*<array>\s*<string>([^<]*)<\/string>/.exec(xml)?.[1];
  const expiresRaw = /<key>ExpirationDate<\/key>\s*<date>([^<]*)<\/date>/.exec(xml)?.[1];
  const certBase64 = /<key>DeveloperCertificates<\/key>\s*<array>\s*<data>([\s\S]*?)<\/data>/.exec(xml)?.[1];
  return {
    path,
    team: teamId,
    appId,
    prefix: appId?.split('.')[0],
    devices: strings('ProvisionedDevices'),
    platforms: strings('Platform'),
    dev: /<key>(?:com\.apple\.security\.)?get-task-allow<\/key>\s*<true\/>/.test(xml),
    expires: new Date(expiresRaw ?? 0),
    developer: extractDeveloperName(certBase64),
  };
}

function findValidProfile(udid: string): Profile | undefined {
  const dirs = [
    ...(process.env.EXACT_PROFILE_DIR ? [resolve(process.env.EXACT_PROFILE_DIR)] : []),
    join(homedir(), 'Library/Developer/Xcode/UserData/Provisioning Profiles'),
    join(homedir(), 'Library/MobileDevice/Provisioning Profiles'),
  ].filter(existsSync);

  const now = new Date();
  const profiles: Profile[] = [];
  for (const dir of dirs) {
    for (const entry of readdirSync(dir)) {
      if (!/\.(mobileprovision|provisionprofile)$/.test(entry)) continue;
      const decoded = decodeProfile(join(dir, entry));
      if (decoded) profiles.push(decoded);
    }
  }

  return profiles
    .filter((p) => {
      if (!p.dev || !(p.expires > now)) return false;
      if (team && p.team !== team) return false;
      if (p.platforms.length > 0 && !p.platforms.includes('iOS')) return false;
      if (!p.devices.includes(udid)) return false;
      return p.appId === `${p.prefix}.${bundleId}` || p.appId === `${p.prefix}.*`;
    })
    .sort((a, b) => b.expires.getTime() - a.expires.getTime())[0];
}

function refreshProvisioningProfile(device: Device) {
  if (!team) {
    throw new Error('DEVELOPMENT_TEAM is not set; cannot refresh provisioning profile automatically.');
  }
  console.log(`Refreshing iOS provisioning profile for ${bundleId} (team ${team}) via Xcode...`);
  const dir = mkdtempSync(join(tmpdir(), 'credits-profile-'));
  try {
    mkdirSync(join(dir, 'Stub'), { recursive: true });
    mkdirSync(join(dir, 'Stub.xcodeproj/xcshareddata/xcschemes'), { recursive: true });

    writeFileSync(
      join(dir, 'Stub/App.swift'),
      'import UIKit\n@main\nclass AppDelegate: UIResponder, UIApplicationDelegate {}\n',
    );

    writeFileSync(
      join(dir, 'Stub.xcodeproj/project.pbxproj'),
      `// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {};
	objectVersion = 56;
	objects = {
		A0000001 = {isa = PBXBuildFile; fileRef = A0000002; };
		A0000002 = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = App.swift; sourceTree = "<group>"; };
		A0000003 = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = Stub.app; sourceTree = BUILT_PRODUCTS_DIR; };
		A0000004 = {isa = PBXFrameworksBuildPhase; buildActionMask = 2147483647; files = (); runOnlyForDeploymentPostprocessing = 0; };
		A0000005 = {isa = PBXGroup; children = (A0000006, A0000007); sourceTree = "<group>"; };
		A0000006 = {isa = PBXGroup; children = (A0000002); path = Stub; sourceTree = "<group>"; };
		A0000007 = {isa = PBXGroup; children = (A0000003); name = Products; sourceTree = "<group>"; };
		A0000008 = {
			isa = PBXNativeTarget;
			buildConfigurationList = A0000012;
			buildPhases = (A0000009, A0000004);
			buildRules = ();
			dependencies = ();
			name = Stub;
			productName = Stub;
			productReference = A0000003;
			productType = "com.apple.product-type.application";
		};
		A0000009 = {isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = (A0000001); runOnlyForDeploymentPostprocessing = 0; };
		A0000010 = {
			isa = PBXProject;
			attributes = {
				BuildIndependentTargetsInParallel = 1;
				LastSwiftUpdateCheck = 1600;
				LastUpgradeCheck = 1600;
				TargetAttributes = {
					A0000008 = { DevelopmentTeam = ${team}; ProvisioningStyle = Automatic; };
				};
			};
			buildConfigurationList = A0000011;
			compatibilityVersion = "Xcode 14.0";
			developmentRegion = en;
			hasScannedForEncodings = 0;
			knownRegions = (en);
			mainGroup = A0000005;
			productRefGroup = A0000007;
			projectDirPath = "";
			projectRoot = "";
			targets = (A0000008);
		};
		A0000011 = {isa = XCConfigurationList; buildConfigurations = (A0000013); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; };
		A0000012 = {isa = XCConfigurationList; buildConfigurations = (A0000014); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; };
		A0000013 = {
			isa = XCBuildConfiguration;
			buildSettings = {
				ALWAYS_SEARCH_USER_PATHS = NO;
				CLANG_ENABLE_MODULES = YES;
				IPHONEOS_DEPLOYMENT_TARGET = 17.0;
				SDKROOT = iphoneos;
				SWIFT_VERSION = 5.0;
			};
			name = Debug;
		};
		A0000014 = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_STYLE = Automatic;
				DEVELOPMENT_TEAM = ${team};
				GENERATE_INFOPLIST_FILE = YES;
				INFOPLIST_KEY_UIApplicationSceneManifest_Generation = YES;
				INFOPLIST_KEY_UILaunchScreen_Generation = YES;
				IPHONEOS_DEPLOYMENT_TARGET = 17.0;
				PRODUCT_BUNDLE_IDENTIFIER = ${bundleId};
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = iphoneos;
				SUPPORTED_PLATFORMS = "iphoneos iphonesimulator";
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2";
			};
			name = Debug;
		};
	};
	rootObject = A0000010;
}
`,
    );

    writeFileSync(
      join(dir, 'Stub.xcodeproj/xcshareddata/xcschemes/Stub.xcscheme'),
      `<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion = "1600" version = "1.7">
   <BuildAction parallelizeBuildables = "YES" buildImplicitDependencies = "YES">
      <BuildActionEntries>
         <BuildActionEntry buildForTesting = "YES" buildForRunning = "YES" buildForProfiling = "YES" buildForArchiving = "YES" buildForAnalyzing = "YES">
            <BuildableReference BuildableIdentifier = "primary" BlueprintIdentifier = "A0000008" BuildableName = "Stub.app" BlueprintName = "Stub" ReferencedContainer = "container:Stub.xcodeproj">
            </BuildableReference>
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
</Scheme>
`,
    );

    const res = spawnSync(
      'xcodebuild',
      [
        '-project',
        join(dir, 'Stub.xcodeproj'),
        '-scheme',
        'Stub',
        '-configuration',
        'Debug',
        '-destination',
        `id=${device.udid}`,
        '-derivedDataPath',
        join(dir, 'DerivedData'),
        '-allowProvisioningUpdates',
        '-allowProvisioningDeviceRegistration',
        'build',
      ],
      { encoding: 'utf8' },
    );

    if (res.status !== 0) {
      const output = [res.stdout, res.stderr].filter(Boolean).join('\n').trim();
      throw new Error(`xcodebuild failed while refreshing the provisioning profile:\n${output}`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function printTrustInstructions(profile?: Profile) {
  const developerLabel = profile?.developer ?? 'Apple Development';
  console.log('');
  if (profile) {
    const expiresLocal = profile.expires.toLocaleString('en-US', {
      dateStyle: 'medium',
      timeStyle: 'short',
    });
    console.log(`Provisioning profile valid until ${expiresLocal} (free-team installs expire after 7 days; rerun \`mise run mobile:phone\` to renew).`);
    console.log('');
  }
  console.log('If iOS shows "Untrusted Developer", approve it once on your iPhone:');
  console.log('  1. Open Settings on your iPhone');
  console.log('  2. Go to General -> VPN & Device Management');
  console.log(`  3. Under Developer App, tap "${developerLabel}"`);
  console.log(`  4. Tap Trust "${developerLabel}" and confirm Trust`);
  console.log(`  5. Open ${appName}`);
}

const device = findPhone(phonePick);
let profile = findValidProfile(device.udid);
if (!profile || forceRefresh) {
  refreshProvisioningProfile(device);
  profile = findValidProfile(device.udid);
  if (!profile) {
    console.error(`Still no valid provisioning profile for ${bundleId} on ${device.name} (${device.udid}).`);
    process.exit(1);
  }
}

const install = spawnSync('bun', ['exact.mjs', 'ios', '--device', ...forwardArgs], {
  cwd: resolve(import.meta.dir, '..'),
  stdio: 'inherit',
});
if (install.status !== 0) {
  process.exit(install.status ?? 1);
}

const launch = spawnSync(
  'xcrun',
  ['devicectl', 'device', 'process', 'launch', '--terminate-existing', '--device', device.udid, bundleId],
  { encoding: 'utf8' },
);

if (launch.status === 0) {
  console.log(`Launched ${appName} on ${device.name}.`);
  printTrustInstructions(profile);
  process.exit(0);
}

const errText = [launch.stdout, launch.stderr].filter(Boolean).join('\n');
if (/explicitly trusted by the user|FBSOpenApplicationErrorDomain error 3/i.test(errText)) {
  console.log(`${appName} is installed on ${device.name}, waiting for developer trust on the phone.`);
  printTrustInstructions(profile);
  process.exit(0);
}
if (/locked/i.test(errText)) {
  console.log(`${appName} is installed on ${device.name}; unlock your iPhone to open it.`);
  printTrustInstructions(profile);
  process.exit(0);
}

console.error(errText.trim() || `Failed to launch ${bundleId} on ${device.name}.`);
printTrustInstructions(profile);
process.exit(launch.status ?? 1);
