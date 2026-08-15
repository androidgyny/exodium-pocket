import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const manifestPath = join(
  process.cwd(),
  "src-tauri",
  "gen",
  "android",
  "app",
  "src",
  "main",
  "AndroidManifest.xml",
);
const buildGradlePath = join(
  process.cwd(),
  "src-tauri",
  "gen",
  "android",
  "app",
  "build.gradle.kts",
);

if (!existsSync(manifestPath)) {
  console.error(
    "AndroidManifest.xml not found. Run `pnpm tauri android init --ci` first.",
  );
  process.exit(1);
}
if (!existsSync(buildGradlePath)) {
  console.error("Android app build.gradle.kts not found. Run `pnpm tauri android init --ci` first.");
  process.exit(1);
}

let xml = readFileSync(manifestPath, "utf8");

const permissionLines = [
  '    <uses-permission android:name="android.permission.MANAGE_EXTERNAL_STORAGE" />',
  '    <uses-permission android:name="android.permission.READ_EXTERNAL_STORAGE" android:maxSdkVersion="32" />',
  '    <uses-permission android:name="android.permission.WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="29" />',
];

const missingPermissions = permissionLines.filter((line) => {
  const match = line.match(/android\.permission\.[A-Z_]+/);
  return match && !xml.includes(match[0]);
});

if (missingPermissions.length > 0) {
  const manifestOpen = xml.match(/<manifest\b[^>]*>\r?\n/);
  if (!manifestOpen || manifestOpen.index === undefined) {
    console.error("Could not find the Android manifest root tag.");
    process.exit(1);
  }
  const insertAt = manifestOpen.index + manifestOpen[0].length;
  xml = `${xml.slice(0, insertAt)}${missingPermissions.join("\n")}\n${xml.slice(insertAt)}`;
}

xml = xml.replace(
  /[ \t]*<uses-permission android:name="android\.permission\.WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="\d+" \/>/,
  '    <uses-permission android:name="android.permission.WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="29" />',
);

if (!xml.includes("android:requestLegacyExternalStorage=")) {
  const beforeTheme = /(\s+android:label="@string\/app_name"\r?\n)/;
  if (!beforeTheme.test(xml)) {
    console.error("Could not find the Android application label attribute.");
    process.exit(1);
  }
  xml = xml.replace(
    beforeTheme,
    '$1        android:requestLegacyExternalStorage="true"\n',
  );
}

writeFileSync(manifestPath, xml);

let gradle = readFileSync(buildGradlePath, "utf8");
gradle = gradle.replace(
  /manifestPlaceholders\["usesCleartextTraffic"\]\s*=\s*"false"/,
  'manifestPlaceholders["usesCleartextTraffic"] = "true"',
);
writeFileSync(buildGradlePath, gradle);

console.log("Patched Android storage and localhost media settings.");
