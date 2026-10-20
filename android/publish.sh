#!/bin/bash
# builds and tests the AARs, then adds this version to site/maven, the tree maven.alkove.ai serves; then commit it there and run site/deploy-maven.sh
set -euo pipefail
cd "$(dirname "$0")"
tree="${SITE:-../../site}/maven"
[ -d "$tree" ] || { echo "no $tree"; exit 1; }
v=$(sed -n 's/^version=//p' gradle.properties)
for a in alkove alkove-livekit; do
  [ ! -e "$tree/ai/alkove/$a/$v" ] || { echo "$a $v is already published: a version is never replaced"; exit 1; }
done
# Gradle adds the new version to the metadata it finds, so the versions already out stay listed
rm -rf build/maven
for a in alkove alkove-livekit; do
  if [ -f "$tree/ai/alkove/$a/maven-metadata.xml" ]; then
    mkdir -p "build/maven/ai/alkove/$a"
    cp "$tree/ai/alkove/$a/maven-metadata.xml" "build/maven/ai/alkove/$a/"
  fi
done
if [ -z "${JAVA_HOME:-}" ] && [ -d "/Applications/Android Studio.app/Contents/jbr/Contents/Home" ]; then
  export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
fi
./gradlew --console=plain -q :alkove:testDebugUnitTest :livekit:testDebugUnitTest publishAllPublicationsToStagingRepository
rsync -a build/maven/ "$tree/"
echo "ai.alkove $v added to $tree"
