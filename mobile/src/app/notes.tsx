// @author wzxgqtl 2026-11-21
// 笔记与备忘列表。快速备忘不取锁，任何情况下都能追加一条。

import { router } from 'expo-router';
import { useEffect, useRef, useState } from 'react';
import {
  FlatList,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { getServerConnection } from '../lib/connection';
import { loadPairedDevice } from '../lib/identity';
import { NotesStore, type ObjectItem } from '../lib/notes';

export default function NotesScreen() {
  const [items, setItems] = useState<ObjectItem[]>([]);
  const [quick, setQuick] = useState('');
  const [error, setError] = useState('');
  const store = useRef<NotesStore | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let active = true;
    let unsubscribe = () => {};
    loadPairedDevice()
      .then((device) => {
        if (!active || !device) {
          if (!device) router.replace('/pair');
          return;
        }
        const connection = getServerConnection(device);
        const notes = new NotesStore(connection, {
          onChanged: () => setItems([...notes.list('note'), ...notes.list('memo')]),
          onError: setError,
          onLockChanged: () => {},
        });
        store.current = notes;
        unsubscribe = connection.onStatus((status) => {
          if (status !== 'connected') return;
          setReady(true);
          notes.pull();
        });
        connection.connect();
      })
      .catch((reason) => setError(String(reason)));
    return () => {
      active = false;
      unsubscribe();
      store.current?.dispose();
    };
  }, []);

  const addQuick = () => {
    const text = quick.trim();
    if (!text || !store.current) return;
    store.current.save({ id: '', kind: 'memo', title: text.slice(0, 40), content: text, baseRev: 0 });
    setQuick('');
  };

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.header}>
        <Pressable onPress={() => router.back()} hitSlop={12}>
          <Text style={styles.back}>返回</Text>
        </Pressable>
        <Text style={styles.title}>笔记</Text>
        <Pressable
          onPress={() => router.push('/note?id=')}
          hitSlop={12}
        >
          <Text style={styles.new}>新建</Text>
        </Pressable>
      </View>

      {error ? (
        <Pressable style={styles.error} onPress={() => setError('')}>
          <Text style={styles.errorText}>{error}</Text>
        </Pressable>
      ) : null}

      <View style={styles.quickRow}>
        <TextInput
          style={styles.quickInput}
          value={quick}
          onChangeText={setQuick}
          placeholder="快速备忘，随手记一句…"
          placeholderTextColor="#87918b"
          onSubmitEditing={addQuick}
          returnKeyType="done"
        />
        <Pressable
          style={[styles.quickButton, !quick.trim() && styles.disabled]}
          onPress={addQuick}
          disabled={!quick.trim()}
        >
          <Text style={styles.quickButtonText}>记下</Text>
        </Pressable>
      </View>

      <FlatList
        data={items}
        keyExtractor={(item) => item.id}
        contentContainerStyle={items.length ? styles.list : styles.empty}
        ListEmptyComponent={
          <Text style={styles.hint}>
            {ready ? '还没有内容。新建一篇，或在上面快速记一条。' : '正在读取电脑上的数据…'}
          </Text>
        }
        renderItem={({ item }) => (
          <Pressable style={styles.card} onPress={() => router.push(`/note?id=${item.id}`)}>
            <View style={styles.cardHead}>
              <Text style={styles.cardTitle} numberOfLines={1}>
                {item.title || '未命名'}
              </Text>
              <Text style={styles.kind}>{item.kind === 'memo' ? '备忘' : '笔记'}</Text>
            </View>
            <Text style={styles.cardBody} numberOfLines={2}>
              {item.content || '（空）'}
            </Text>
          </Pressable>
        )}
      />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f5f1e8' },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 20,
    paddingVertical: 14,
  },
  back: { color: '#65736c', fontSize: 14 },
  title: { color: '#23362f', fontSize: 20, fontWeight: '800' },
  new: { color: '#547665', fontSize: 14, fontWeight: '700' },
  error: { backgroundColor: '#f7e5df', marginHorizontal: 16, padding: 12, borderRadius: 12 },
  errorText: { color: '#834d43', lineHeight: 20 },
  quickRow: { flexDirection: 'row', gap: 10, paddingHorizontal: 16, paddingBottom: 8 },
  quickInput: {
    flex: 1,
    minHeight: 44,
    borderRadius: 14,
    backgroundColor: '#fffdf8',
    color: '#23362f',
    fontSize: 15,
    paddingHorizontal: 14,
  },
  quickButton: {
    minWidth: 62,
    borderRadius: 14,
    backgroundColor: '#547665',
    alignItems: 'center',
    justifyContent: 'center',
  },
  quickButtonText: { color: '#fffdf8', fontWeight: '700' },
  disabled: { opacity: 0.45 },
  list: { padding: 16, gap: 12 },
  empty: { flexGrow: 1, justifyContent: 'center', padding: 32 },
  hint: { color: '#65736c', fontSize: 15, lineHeight: 23, textAlign: 'center' },
  card: { backgroundColor: '#fffdf8', borderRadius: 18, padding: 16 },
  cardHead: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 10 },
  cardTitle: { flex: 1, color: '#23362f', fontSize: 16, fontWeight: '700' },
  kind: { color: '#87918b', fontSize: 12 },
  cardBody: { color: '#65736c', fontSize: 14, lineHeight: 21, marginTop: 6 },
});
