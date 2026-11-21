// @author wzxgqtl 2026-11-21
// 产物详情：图片直接渲染，文本类拉取后展示，其余交给系统应用。
//
// 不渲染 HTML、不执行任何脚本：产物内容一律当纯文本处理，避免注入。

import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Image,
  Linking,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { getServerConnection } from '../lib/connection';
import { formatSize } from '../lib/format';
import { loadPairedDevice, type PairedDevice } from '../lib/identity';
import { NotesStore, artifactUrl, type ObjectItem } from '../lib/notes';

const MAX_INLINE_BYTES = 512 * 1024;

export default function ArtifactScreen() {
  const params = useLocalSearchParams<{ id?: string }>();
  const objectId = params.id ?? '';

  const [device, setDevice] = useState<PairedDevice | null>(null);
  const [item, setItem] = useState<ObjectItem | null>(null);
  const [text, setText] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    let active = true;
    let unsubscribe = () => {};
    let store: NotesStore | null = null;

    loadPairedDevice()
      .then((paired) => {
        if (!active) return;
        if (!paired) {
          router.replace('/pair');
          return;
        }
        setDevice(paired);
        const connection = getServerConnection(paired);
        store = new NotesStore(connection, {
          onChanged: () => {
            const latest = store?.byId(objectId);
            if (active && latest) setItem(latest);
          },
          onError: setError,
          onLockChanged: () => {},
        });
        unsubscribe = connection.onStatus((status) => {
          if (status === 'connected') store?.pull();
        });
        connection.connect();
      })
      .catch((reason) => setError(String(reason)));

    return () => {
      active = false;
      unsubscribe();
      store?.dispose();
    };
  }, [objectId]);

  const mime = item?.artifact?.mime ?? '';
  const blobId = item?.artifact?.blobId ?? '';
  const isImage = mime.startsWith('image/');
  const isText = mime.startsWith('text/') || mime === 'application/json';
  // 大文件不内联渲染：这类判断是纯派生值，不该用 state 存。
  const tooLarge = isText && (item?.artifact?.size ?? 0) > MAX_INLINE_BYTES;

  useEffect(() => {
    if (!device || !blobId || !isText || tooLarge) return;
    let active = true;
    fetch(artifactUrl(device, blobId))
      .then((response) => (response.ok ? response.text() : Promise.reject(new Error('下载失败'))))
      .then((value) => {
        if (active) setText(value);
      })
      .catch((reason) => {
        if (active) setError(String(reason));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [device, blobId, isText, tooLarge]);

  const openExternally = () => {
    if (!device || !item?.artifact) return;
    Linking.openURL(artifactUrl(device, item.artifact.blobId)).catch(() =>
      setError('没有可以打开这个文件的应用'),
    );
  };

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.header}>
        <Pressable onPress={() => router.back()} hitSlop={12}>
          <Text style={styles.back}>返回</Text>
        </Pressable>
        <Text style={styles.title} numberOfLines={1}>
          {item?.artifact?.name ?? '产物'}
        </Text>
        <Pressable onPress={openExternally} hitSlop={12}>
          <Text style={styles.open}>打开</Text>
        </Pressable>
      </View>

      {item?.artifact ? (
        <Text style={styles.meta}>
          {mime || '未知类型'} · {formatSize(item.artifact.size)}
        </Text>
      ) : null}

      {error ? (
        <Pressable style={styles.error} onPress={() => setError('')}>
          <Text style={styles.errorText}>{error}</Text>
        </Pressable>
      ) : null}

      {loading && !isImage && !tooLarge ? (
        <ActivityIndicator style={styles.spinner} color="#547665" />
      ) : null}

      {isImage && device && blobId ? (
        <Image
          source={{ uri: artifactUrl(device, blobId) }}
          style={styles.image}
          resizeMode="contain"
        />
      ) : null}

      {isText && !tooLarge ? (
        <ScrollView contentContainerStyle={styles.textBody}>
          <Text style={styles.mono}>{text || (loading ? '正在读取…' : '（空文件）')}</Text>
        </ScrollView>
      ) : null}

      {tooLarge ? (
        <View style={styles.fallback}>
          <Text style={styles.hint}>文件较大，不内联预览。</Text>
          <Pressable style={styles.button} onPress={openExternally}>
            <Text style={styles.buttonText}>用系统应用打开</Text>
          </Pressable>
        </View>
      ) : null}

      {!isImage && !isText ? (
        <View style={styles.fallback}>
          <Text style={styles.hint}>这类文件不在应用内预览。</Text>
          <Pressable style={styles.button} onPress={openExternally}>
            <Text style={styles.buttonText}>用系统应用打开</Text>
          </Pressable>
        </View>
      ) : null}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f5f1e8' },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 12,
    paddingHorizontal: 20,
    paddingVertical: 14,
  },
  back: { color: '#65736c', fontSize: 14 },
  title: { flex: 1, color: '#23362f', fontSize: 16, fontWeight: '700' },
  open: { color: '#547665', fontSize: 14, fontWeight: '700' },
  meta: { color: '#87918b', fontSize: 12, paddingHorizontal: 20, paddingBottom: 10 },
  error: { backgroundColor: '#f7e5df', marginHorizontal: 16, padding: 12, borderRadius: 12 },
  errorText: { color: '#834d43', lineHeight: 20 },
  spinner: { marginTop: 24 },
  image: { flex: 1, margin: 16, borderRadius: 14 },
  textBody: { padding: 20 },
  mono: { color: '#3c4a43', fontSize: 13, lineHeight: 21, fontFamily: 'monospace' },
  fallback: { flex: 1, alignItems: 'center', justifyContent: 'center', gap: 16, padding: 32 },
  hint: { color: '#65736c', fontSize: 15 },
  button: {
    backgroundColor: '#547665',
    paddingHorizontal: 20,
    paddingVertical: 12,
    borderRadius: 14,
  },
  buttonText: { color: '#fffdf8', fontWeight: '700' },
});
