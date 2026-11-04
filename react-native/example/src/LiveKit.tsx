import { AudioSession } from '@livekit/react-native';
import { AlkoveFocus } from '@alkove/react-native-livekit';
import { LocalAudioTrack, Room, Track } from 'livekit-client';
import { useEffect, useRef, useState } from 'react';
import { Button, Platform, Text, TextInput } from 'react-native';

// a LiveKit room, the microphone published through Focus: the frames it processed show it is in the path
export function LiveKit() {
  const [url, setUrl] = useState(Platform.OS === 'android' ? 'ws://10.0.2.2:7880' : 'ws://localhost:7880');
  const [token, setToken] = useState('');
  const [state, setState] = useState('off');
  const [frames, setFrames] = useState(0);
  const room = useRef<Room | null>(null);
  const focus = useRef(AlkoveFocus());

  useEffect(() => {
    const t = setInterval(() => setFrames(focus.current.processed()), 500);
    return () => clearInterval(t);
  }, []);

  const connect = async () => {
    try {
      setState('connecting');
      // iOS: registerGlobals() sets the audio session up with WebRTC's engine
      if (Platform.OS === 'android') await AudioSession.startAudioSession();
      const r = new Room();
      room.current = r;
      await r.connect(url, token);
      await r.localParticipant.setMicrophoneEnabled(true);
      const track = r.localParticipant.getTrackPublication(Track.Source.Microphone)?.audioTrack;
      if (!(track instanceof LocalAudioTrack)) throw new Error('no microphone track');
      await track.setProcessor(focus.current);
      setState('connected');
    } catch (e) {
      setState(`error ${String(e)}`);
    }
  };

  const disconnect = async () => {
    await room.current?.disconnect();
    room.current = null;
    if (Platform.OS === 'android') await AudioSession.stopAudioSession();
    setState('off');
  };

  return (
    <>
      <TextInput testID="url" value={url} onChangeText={setUrl} autoCapitalize="none" />
      <TextInput testID="token" value={token} onChangeText={setToken} autoCapitalize="none" placeholder="token" />
      {state === 'connected' ? <Button testID="leave" title="Leave" onPress={disconnect} /> : <Button testID="join" title="Join" onPress={connect} />}
      <Text testID="state">{state}</Text>
      <Text testID="frames">{frames > 0 ? `focus on the mic, ${frames} frames` : 'no frame yet'}</Text>
    </>
  );
}
