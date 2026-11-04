const {
  createRunOncePlugin,
  withInfoPlist,
  withProjectBuildGradle,
} = require('expo/config-plugins');

const pkg = require('./package.json');

const MAVEN = 'https://maven.alkove.ai';
const MICROPHONE = 'Listens for your voice on this device.';

const withMicrophone = (config, { microphonePermission } = {}) =>
  withInfoPlist(config, (c) => {
    c.modResults.NSMicrophoneUsageDescription =
      microphonePermission ||
      c.modResults.NSMicrophoneUsageDescription ||
      MICROPHONE;
    return c;
  });

const withMaven = (config) =>
  withProjectBuildGradle(config, (c) => {
    if (c.modResults.language !== 'groovy') {
      throw new Error(
        `@alkove/react-native: add the repository ${MAVEN} to android/build.gradle.kts by hand`
      );
    }
    if (!c.modResults.contents.includes(MAVEN)) {
      const anchor = /allprojects\s*\{\s*repositories\s*\{/;
      if (!anchor.test(c.modResults.contents)) {
        throw new Error(
          `@alkove/react-native: no allprojects.repositories in android/build.gradle, add maven { url '${MAVEN}' } by hand`
        );
      }
      c.modResults.contents = c.modResults.contents.replace(
        anchor,
        (m) => `${m}\n    maven { url '${MAVEN}' }`
      );
    }
    return c;
  });

const withAlkove = (config, props) => withMaven(withMicrophone(config, props));

module.exports = createRunOncePlugin(withAlkove, pkg.name, pkg.version);
