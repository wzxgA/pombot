// @author wzxgqtl 2026-11-21
// 产物详情：图片直接渲染，文本类拉取后展示，其余交给系统应用。
//
// 不渲染 HTML、不执行任何脚本：产物内容一律当纯文本处理，避免注入。

import Markdown from '@ronradtke/react-native-markdown-display';
import { File, Paths } from 'expo-file-system';
import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import Pdf from 'react-native-pdf';
import { WebView } from 'react-native-webview';
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
/** Cloudflare 免费版对单个响应体的上限，超过会直接失败。 */
const LARGE_FILE_BYTES = 100 * 1024 * 1024;

export default function ArtifactScreen() {
  const params = useLocalSearchParams<{ id?: string }>();
  const objectId = params.id ?? '';

  const [device, setDevice] = useState<PairedDevice | null>(null);
  const [item, setItem] = useState<ObjectItem | null>(null);
  const [text, setText] = useState('');
  const [localPdfUri, setLocalPdfUri] = useState<string | null>(null);
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
  const isPdf = mime === 'application/pdf' || item?.artifact?.name.toLowerCase().endsWith('.pdf');
  const isHtml = mime === 'text/html' || item?.artifact?.name.toLowerCase().endsWith('.html') || item?.artifact?.name.toLowerCase().endsWith('.htm');
  const isMarkdown =
    mime === 'text/markdown' ||
    item?.artifact?.name.toLowerCase().endsWith('.md') ||
    item?.artifact?.name.toLowerCase().endsWith('.markdown');
  const isText = (mime.startsWith('text/') || mime === 'application/json') && !isMarkdown;
  // 大文件不内联渲染：这类判断是纯派生值，不该用 state 存。
  const tooLarge = (isText || isMarkdown || isHtml) && (item?.artifact?.size ?? 0) > MAX_INLINE_BYTES;
  const pdfTooLarge = isPdf && (item?.artifact?.size ?? 0) > LARGE_FILE_BYTES;
  const artifactUri = device && blobId ? artifactUrl(getServerConnection(device).baseUrl, device, blobId) : '';

  useEffect(() => {
    if (!device || !blobId || (!isText && !isMarkdown && !isHtml) || tooLarge) return;
    let active = true;
    fetch(artifactUrl(getServerConnection(device).baseUrl, device, blobId))
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
  }, [device, blobId, isText, isMarkdown, isHtml, tooLarge]);

  useEffect(() => {
    if (!device || !blobId || !isPdf || pdfTooLarge || localPdfUri) return;
    const target = new File(Paths.cache, `pombot-${blobId}.pdf`);
    let active = true;
    File.downloadFileAsync(artifactUri, target, { idempotent: true })
      .then((file) => {
        if (active) setLocalPdfUri(file.uri);
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
  }, [device, blobId, isPdf, pdfTooLarge, localPdfUri, artifactUri]);

  const openExternally = () => {
    if (!device || !item?.artifact) return;
    Linking.openURL(
      artifactUrl(getServerConnection(device).baseUrl, device, item.artifact.blobId),
    ).catch(() =>
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

      {item?.artifact && item.artifact.size > LARGE_FILE_BYTES ? (
        <Text style={styles.warn}>
          超过 100 MB，经公网隧道可能下载失败，建议在同一 WiFi 下打开
        </Text>
      ) : null}

      {error ? (
        <Pressable style={styles.error} onPress={() => setError('')}>
          <Text style={styles.errorText}>{error}</Text>
        </Pressable>
      ) : null}

      {loading && !isImage && !isPdf && !tooLarge ? (
        <ActivityIndicator style={styles.spinner} color="#547665" />
      ) : null}

      {isImage && artifactUri ? (
        <Image source={{ uri: artifactUri }} style={styles.image} resizeMode="contain" />
      ) : null}

      {isPdf && localPdfUri && !pdfTooLarge ? (
        <Pdf source={{ uri: localPdfUri }} style={styles.pdf} trustAllCerts={false} />
      ) : null}

      {isHtml && !tooLarge && artifactUri ? (
        <WebView source={{ uri: artifactUri }} style={styles.webView} originWhitelist={['*']} javaScriptEnabled={false} />
      ) : null}

      {isMarkdown && !tooLarge ? (
        <ScrollView contentContainerStyle={styles.markdownBody}>
          {text ? <Markdown style={markdownStyles}>{text}</Markdown> : <Text style={styles.mono}>{loading ? '正在读取…' : '（空文件）'}</Text>}
        </ScrollView>
      ) : null}

      {isText && !tooLarge ? (
        <ScrollView contentContainerStyle={styles.textBody}>
          <Text style={styles.mono}>{text || (loading ? '正在读取…' : '（空文件）')}</Text>
        </ScrollView>
      ) : null}

      {(tooLarge || pdfTooLarge) ? (
        <View style={styles.fallback}>
          <Text style={styles.hint}>文件较大，不内联预览。</Text>
          <Pressable style={styles.button} onPress={openExternally}>
            <Text style={styles.buttonText}>用系统应用打开</Text>
          </Pressable>
        </View>
      ) : null}

      {!isImage && !isPdf && !isHtml && !isText && !isMarkdown ? (
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
  warn: {
    color: '#8a5b4a',
    fontSize: 12,
    lineHeight: 18,
    paddingHorizontal: 20,
    paddingBottom: 10,
  },
  error: { backgroundColor: '#f7e5df', marginHorizontal: 16, padding: 12, borderRadius: 12 },
  errorText: { color: '#834d43', lineHeight: 20 },
  spinner: { marginTop: 24 },
  image: { flex: 1, margin: 16, borderRadius: 14 },
  pdf: { flex: 1, width: '100%' },
  webView: { flex: 1, backgroundColor: '#fffdf8' },
  textBody: { padding: 20 },
  markdownBody: { padding: 20, paddingBottom: 40 },
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

const markdownStyles = {
  body: { color: '#3c4a43', fontSize: 15, lineHeight: 24 },
  heading1: { color: '#23362f', fontSize: 26, fontWeight: '800' as const, marginTop: 8, marginBottom: 12 },
  heading2: { color: '#23362f', fontSize: 21, fontWeight: '800' as const, marginTop: 18, marginBottom: 8 },
  heading3: { color: '#23362f', fontSize: 18, fontWeight: '700' as const, marginTop: 14, marginBottom: 6 },
  paragraph: { marginTop: 0, marginBottom: 12 },
  strong: { color: '#23362f', fontWeight: '800' as const },
  em: { fontStyle: 'italic' as const },
  link: { color: '#547665', textDecorationLine: 'underline' as const },
  blockquote: { backgroundColor: '#e8eee8', borderLeftColor: '#547665', borderLeftWidth: 4, paddingHorizontal: 12, paddingVertical: 6 },
  code_inline: { backgroundColor: '#ece8de', color: '#834d43', borderRadius: 4, paddingHorizontal: 4 },
  code_block: { backgroundColor: '#23362f', color: '#fffdf8', borderRadius: 10, padding: 12, fontFamily: 'monospace' },
  bullet_list: { marginBottom: 8 },
  ordered_list: { marginBottom: 8 },
  list_item: { marginBottom: 4 },
};
