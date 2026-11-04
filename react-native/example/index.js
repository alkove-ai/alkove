import { registerGlobals } from '@livekit/react-native';
import { AppRegistry } from 'react-native';
import App from './src/App';
import { name as appName } from './app.json';

registerGlobals();

AppRegistry.registerComponent(appName, () => App);
