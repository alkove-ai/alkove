import { Spot, useWakeListener, type Lang, type Word } from '@alkove/react-native';
import { useState } from 'react';
import { Button, FlatList, SafeAreaView, StyleSheet, Text, TextInput, View } from 'react-native';

import { Agent } from './Agent';
import { LiveKit } from './LiveKit';

const LANGS: Lang[] = ['en', 'fr', 'es', 'it', 'pt', 'ca', 'ro'];

export default function App() {
  const [screen, setScreen] = useState<'wake' | 'agent' | 'livekit'>('wake');
  return (
    <SafeAreaView style={styles.page}>
      <View style={styles.row}>
        <Button testID="screen-wake" title="Wake word" onPress={() => setScreen('wake')} />
        <Button testID="screen-agent" title="Agent" onPress={() => setScreen('agent')} />
        <Button testID="screen-livekit" title="LiveKit" onPress={() => setScreen('livekit')} />
      </View>
      {screen === 'wake' ? <Wake /> : screen === 'agent' ? <Agent /> : <LiveKit />}
    </SafeAreaView>
  );
}

function Wake() {
  const [text, setText] = useState('Hey Nova');
  const [lang, setLang] = useState<Lang>('en');
  const [word, setWord] = useState<Word | null>(null);
  const [wakes, setWakes] = useState<string[]>([]);
  const [failure, setFailure] = useState<string | null>(null);
  const { status, error } = useWakeListener({
    enabled: !!word,
    words: word ? [word] : [],
    on: { wake: (w) => setWakes((l) => [`${new Date().toLocaleTimeString()}: « ${w.word} »`, ...l]) },
  });

  const listen = async () => {
    setFailure(null);
    try {
      setWord(await Spot.compile(text, { lang }));
    } catch (e) {
      setFailure(String(e));
    }
  };

  return (
    <>
      <Text style={styles.title}>Wake word</Text>
      <TextInput testID="word" style={styles.input} value={text} onChangeText={setText} />
      <View style={styles.row}>
        {LANGS.map((l) => (
          <Button key={l} title={l === lang ? `[${l}]` : l} onPress={() => setLang(l)} />
        ))}
      </View>
      {word ? <Button testID="stop" title="Stop" onPress={() => setWord(null)} /> : <Button testID="listen" title="Listen" onPress={listen} />}
      <Text testID="status">{status}</Text>
      {word?.warnings.map((w) => <Text key={w.code}>{w.message}</Text>)}
      {(error || failure) && <Text testID="error">{error ? `${(error as { code?: string }).code ?? ''} ${error.message}` : failure}</Text>}
      <FlatList testID="wakes" data={wakes} keyExtractor={(w, i) => `${i}${w}`} renderItem={({ item }) => <Text>{item}</Text>} />
    </>
  );
}

const styles = StyleSheet.create({
  page: { flex: 1, padding: 16, paddingTop: 48, gap: 8 },
  title: { fontSize: 22, fontWeight: '600' },
  input: { borderWidth: 1, borderColor: '#999', padding: 8, fontSize: 16 },
  row: { flexDirection: 'row', flexWrap: 'wrap' },
});
