import { useAgentInput } from '@alkove/react-native';
import { useState } from 'react';
import { Button, FlatList, Text } from 'react-native';

// each sentence and each end of turn, as the web example next-agent shows them
export function Agent() {
  const [on, setOn] = useState(false);
  const [items, setItems] = useState<{ at: number; text: string }[]>([]);
  const add = (at: number, text: string) => setItems((l) => [...l, { at, text }].sort((a, b) => a.at - b.at));
  const { status, mode, error } = useAgentInput({
    enabled: on,
    on: {
      utterance: ({ pcm, start, end }) => add(start, `${(end - start).toFixed(1)} s, ${pcm.byteLength} bytes`),
      turnend: ({ end, probability }) => add(end, `end of turn (${probability.toFixed(2)})`),
    },
  });
  return (
    <>
      <Button testID="talk" title={on ? 'Stop' : 'Talk'} onPress={() => setOn(!on)} />
      <Text testID="status">{status}</Text>
      <Text testID="mode">{mode ?? '-'}</Text>
      {error && <Text testID="error">{`${(error as { code?: string }).code ?? ''} ${error.message}`}</Text>}
      <FlatList data={items} keyExtractor={(it, i) => `${i}${it.at}`} renderItem={({ item }) => <Text>{item.text}</Text>} />
    </>
  );
}
