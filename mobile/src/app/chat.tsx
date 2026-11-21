import { router } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  FlatList,
  KeyboardAvoidingView,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import Markdown from '@ronradtke/react-native-markdown-display';

import { Pom } from '../components/Pom';
import { ChatStore, type ChatMessage, type ConnectionStatus } from '../lib/chat';
import { getServerConnection, resetServerConnection } from '../lib/connection';
import { clearPairedDevice, loadPairedDevice } from '../lib/identity';

export default function ChatScreen() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [status, setStatus] = useState<ConnectionStatus>('connecting');
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState('');
  const store = useRef<ChatStore | null>(null);
  const list = useRef<FlatList<ChatMessage>>(null);

  useEffect(() => {
    let active = true;
    let unsubscribe = () => {};
    loadPairedDevice()
      .then((device) => {
        if (!active) return;
        if (!device) {
          router.replace('/pair');
          return;
        }
        const connection = getServerConnection(device);
        store.current = new ChatStore(connection, {
          onMessages: setMessages,
          onSending: setSending,
          onError: setError,
        });
        unsubscribe = connection.onStatus(setStatus);
        connection.connect();
      })
      .catch((reason) => setError(String(reason)));
    return () => {
      active = false;
      unsubscribe();
      store.current?.dispose();
    };
  }, []);

  const send = () => {
    if (!input.trim() || sending) return;
    try {
      setError('');
      store.current?.sendMessage(input);
      setInput('');
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const unpair = () => {
    Alert.alert('解除这台手机的配对？', '聊天记录仍保留在电脑上。', [
      { text: '取消', style: 'cancel' },
      {
        text: '解除',
        style: 'destructive',
        onPress: () => {
          resetServerConnection();
          clearPairedDevice().finally(() => router.replace('/pair'));
        },
      },
    ]);
  };

  const connected = status === 'connected';
  return (
    <SafeAreaView style={styles.safe}>
      <StatusBar style="dark" />
      <View style={styles.header}>
        <View>
          <Text style={styles.title}>PomBot</Text>
          <Text style={[styles.status, connected ? styles.online : styles.offline]}>
            {connected ? '已连接（局域网）' : status === 'connecting' ? '正在连接电脑…' : '离线，正在重连'}
          </Text>
        </View>
        <View style={styles.headerActions}>
          <Pressable onPress={() => router.push('/notes')} hitSlop={10}>
            <Text style={styles.nav}>笔记</Text>
          </Pressable>
          <Pressable onPress={() => router.push('/artifacts')} hitSlop={10}>
            <Text style={styles.nav}>产物</Text>
          </Pressable>
          <Pressable onPress={unpair} hitSlop={12}>
            <Text style={styles.unpair}>解除配对</Text>
          </Pressable>
        </View>
      </View>
      {error ? <Pressable style={styles.error} onPress={() => setError('')}><Text style={styles.errorText}>{error}</Text></Pressable> : null}
      <KeyboardAvoidingView style={styles.body} behavior="padding">
        <FlatList
          ref={list}
          data={messages}
          keyExtractor={(item) => item.id}
          contentContainerStyle={messages.length ? styles.messages : styles.empty}
          onContentSizeChange={() => list.current?.scrollToEnd({ animated: true })}
          ListEmptyComponent={<View style={styles.welcome}><Pom offline={!connected} size={136} /><Text style={styles.hello}>今天想做点什么？</Text><Text style={styles.hint}>说一句话，Pom 会通过电脑上的模型回答你。</Text></View>}
          renderItem={({ item }) => (
            <View style={[styles.bubble, item.role === 'user' ? styles.userBubble : styles.assistantBubble]}>
              {item.role === 'user' || item.streaming ? (
                // 用户输入按原样显示，避免手动打的符号被 Markdown 语法吞掉；
                // 流式阶段同样走纯文本，收完再一次性解析，避免每帧重排。
                <Text style={item.role === 'user' ? styles.userText : styles.assistantText}>
                  {item.content || '…'}
                </Text>
              ) : (
                <Markdown style={markdownStyles}>{item.content || '…'}</Markdown>
              )}
            </View>
          )}
        />
        <View style={styles.composer}>
          <TextInput
            style={styles.input}
            value={input}
            onChangeText={setInput}
            placeholder="给 Pom 发消息…"
            placeholderTextColor="#87918b"
            multiline
            maxLength={32000}
            editable={!sending}
          />
          <Pressable style={[styles.send, (!input.trim() || sending || !connected) && styles.sendDisabled]} onPress={send} disabled={!input.trim() || sending || !connected}>
            {sending ? <ActivityIndicator color="#fffdf8" /> : <Text style={styles.sendText}>发送</Text>}
          </Pressable>
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

// 助手回复按 Markdown 渲染。配色沿用纸感米白 + 深橄榄绿，
// 代码块用浅底等宽字体，保证长段输出仍然可读。
// @author wzxgqtl 2026-11-21
const markdownStyles = {
  body: { color: '#23362f', fontSize: 16, lineHeight: 24 },
  paragraph: { marginTop: 0, marginBottom: 8 },
  heading1: { color: '#23362f', fontSize: 20, fontWeight: '800' as const, marginTop: 8, marginBottom: 6 },
  heading2: { color: '#23362f', fontSize: 18, fontWeight: '800' as const, marginTop: 8, marginBottom: 6 },
  heading3: { color: '#23362f', fontSize: 17, fontWeight: '700' as const, marginTop: 6, marginBottom: 4 },
  strong: { fontWeight: '700' as const },
  em: { fontStyle: 'italic' as const },
  link: { color: '#3d6b57', textDecorationLine: 'underline' as const },
  blockquote: {
    backgroundColor: '#f0ece1',
    borderLeftColor: '#547665',
    borderLeftWidth: 3,
    paddingHorizontal: 10,
    paddingVertical: 6,
    marginVertical: 6,
  },
  code_inline: {
    backgroundColor: '#f0ece1',
    color: '#7a4a3c',
    fontFamily: 'monospace',
    fontSize: 14,
    paddingHorizontal: 4,
    borderRadius: 4,
  },
  code_block: {
    backgroundColor: '#f0ece1',
    color: '#33413a',
    fontFamily: 'monospace',
    fontSize: 13,
    lineHeight: 20,
    padding: 10,
    borderRadius: 10,
    marginVertical: 6,
  },
  fence: {
    backgroundColor: '#f0ece1',
    color: '#33413a',
    fontFamily: 'monospace',
    fontSize: 13,
    lineHeight: 20,
    padding: 10,
    borderRadius: 10,
    marginVertical: 6,
  },
  bullet_list: { marginVertical: 4 },
  ordered_list: { marginVertical: 4 },
  list_item: { marginVertical: 2 },
  hr: { backgroundColor: '#e0dbcf', height: 1, marginVertical: 10 },
};

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f5f1e8' },
  header: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', paddingHorizontal: 22, paddingVertical: 16 },
  title: { fontSize: 25, fontWeight: '800', color: '#23362f' },
  status: { marginTop: 3, fontSize: 13 },
  online: { color: '#547665' },
  offline: { color: '#9a685e' },
  headerActions: { flexDirection: 'row', alignItems: 'center', gap: 14 },
  nav: { color: '#547665', fontSize: 14, fontWeight: '600' },
  unpair: { color: '#65736c', fontSize: 13 },
  error: { backgroundColor: '#f7e5df', marginHorizontal: 16, padding: 12, borderRadius: 12 },
  errorText: { color: '#834d43', lineHeight: 20 },
  body: { flex: 1 },
  messages: { padding: 18, gap: 12 },
  empty: { flexGrow: 1, justifyContent: 'center', padding: 32 },
  welcome: { alignItems: 'center' },
  hello: { color: '#23362f', fontSize: 24, fontWeight: '700', marginTop: 22 },
  hint: { color: '#65736c', fontSize: 15, lineHeight: 23, textAlign: 'center', marginTop: 10 },
  bubble: { maxWidth: '86%', paddingHorizontal: 16, paddingVertical: 12, borderRadius: 20 },
  userBubble: { alignSelf: 'flex-end', backgroundColor: '#547665', borderBottomRightRadius: 5 },
  assistantBubble: { alignSelf: 'flex-start', backgroundColor: '#fffdf8', borderBottomLeftRadius: 5 },
  userText: { color: '#fffdf8', fontSize: 16, lineHeight: 24 },
  assistantText: { color: '#23362f', fontSize: 16, lineHeight: 24 },
  composer: { flexDirection: 'row', alignItems: 'flex-end', gap: 10, padding: 14, backgroundColor: '#fffdf8', borderTopColor: '#e6e2d8', borderTopWidth: 1 },
  input: { flex: 1, minHeight: 46, maxHeight: 130, borderRadius: 16, backgroundColor: '#f5f1e8', color: '#23362f', fontSize: 16, paddingHorizontal: 15, paddingVertical: 11 },
  send: { height: 46, minWidth: 66, borderRadius: 16, backgroundColor: '#547665', alignItems: 'center', justifyContent: 'center', paddingHorizontal: 15 },
  sendDisabled: { opacity: 0.45 },
  sendText: { color: '#fffdf8', fontSize: 15, fontWeight: '700' },
});
