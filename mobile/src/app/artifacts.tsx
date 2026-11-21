// @author wzxgqtl 2026-11-21
// 产物列表。只同步元数据，缩略图按需拉取；点开才下载原文件。

import { router } from 'expo-router';
import { useEffect, useRef, useState } from 'react';
import { FlatList, Image, Pressable, StyleSheet, Text, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { getServerConnection } from '../lib/connection';
import { extensionOf, formatSize } from '../lib/format';
import { loadPairedDevice, type PairedDevice } from '../lib/identity';
import { NotesStore, artifactUrl, type ObjectItem } from '../lib/notes';

export default function ArtifactsScreen() {
  const [items, setItems] = useState<ObjectItem[]>([]);
  const [error, setError] = useState('');
  const [device, setDevice] = useState<PairedDevice | null>(null);
  const [ready, setReady] = useState(false);
  const store = useRef<NotesStore | null>(null);

  useEffect(() => {
    let active = true;
    let unsubscribe = () => {};
    loadPairedDevice()
      .then((paired) => {
        if (!active) return;
        if (!paired) {
          router.replace('/pair');
          return;
        }
        setDevice(paired);
        const connection = getServerConnection(paired);
        const notes = new NotesStore(connection, {
          onChanged: () => setItems(notes.list('artifact')),
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

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.header}>
        <Pressable onPress={() => router.back()} hitSlop={12}>
          <Text style={styles.back}>返回</Text>
        </Pressable>
        <Text style={styles.title}>产物</Text>
        <Text style={styles.count}>{items.length}</Text>
      </View>

      {error ? (
        <Pressable style={styles.error} onPress={() => setError('')}>
          <Text style={styles.errorText}>{error}</Text>
        </Pressable>
      ) : null}

      <FlatList
        data={items}
        keyExtractor={(item) => item.id}
        numColumns={2}
        columnWrapperStyle={items.length ? styles.row : undefined}
        contentContainerStyle={items.length ? styles.grid : styles.empty}
        ListEmptyComponent={
          <Text style={styles.hint}>
            {ready
              ? '还没有产物。电脑上执行 pombotd.exe --add-artifact <文件> 可以登记一个。'
              : '正在读取…'}
          </Text>
        }
        renderItem={({ item }) => {
          const thumb =
            device && item.artifact
              ? artifactUrl(getServerConnection(device).baseUrl, device, item.artifact.blobId, true)
              : null;
          const isImage = item.artifact?.mime.startsWith('image/');
          return (
            <Pressable style={styles.card} onPress={() => router.push(`/artifact?id=${item.id}`)}>
              <View style={styles.preview}>
                {isImage && thumb ? (
                  <Image source={{ uri: thumb }} style={styles.thumb} resizeMode="cover" />
                ) : (
                  <Text style={styles.ext}>{extensionOf(item.artifact?.name ?? item.title)}</Text>
                )}
              </View>
              <Text style={styles.name} numberOfLines={1}>
                {item.artifact?.name ?? item.title}
              </Text>
              <Text style={styles.size}>{formatSize(item.artifact?.size ?? 0)}</Text>
            </Pressable>
          );
        }}
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
  count: { color: '#87918b', fontSize: 13 },
  error: { backgroundColor: '#f7e5df', marginHorizontal: 16, padding: 12, borderRadius: 12 },
  errorText: { color: '#834d43', lineHeight: 20 },
  grid: { padding: 14, gap: 12 },
  row: { gap: 12 },
  empty: { flexGrow: 1, justifyContent: 'center', padding: 32 },
  hint: { color: '#65736c', fontSize: 15, lineHeight: 23, textAlign: 'center' },
  card: { flex: 1, backgroundColor: '#fffdf8', borderRadius: 16, padding: 10 },
  preview: {
    height: 110,
    borderRadius: 12,
    backgroundColor: '#f0ece1',
    alignItems: 'center',
    justifyContent: 'center',
    overflow: 'hidden',
  },
  thumb: { width: '100%', height: '100%' },
  ext: { color: '#8a958f', fontSize: 15, fontWeight: '700', letterSpacing: 1 },
  name: { color: '#23362f', fontSize: 13, fontWeight: '600', marginTop: 8 },
  size: { color: '#87918b', fontSize: 11, marginTop: 2 },
});
