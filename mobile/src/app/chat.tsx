import { router } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  FlatList,
  KeyboardAvoidingView,
  Modal,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import Markdown from '@ronradtke/react-native-markdown-display';

import { Pom } from '../components/Pom';
import type { ToolConfirmPayload } from '../generated/contract';
import { ChatStore, type ChatMessage, type ConnectionStatus } from '../lib/chat';
import { getServerConnection, resetServerConnection, type ServerConnection } from '../lib/connection';
import { clearPairedDevice, loadPairedDevice } from '../lib/identity';

interface PendingConfirm {
  callId: string;
  tool: string;
  summary: string;
  expiresIn: number;
}

export default function ChatScreen() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [status, setStatus] = useState<ConnectionStatus>('connecting');
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState('');
  const [network, setNetwork] = useState<'lan' | 'public' | null>(null);
  const [confirm, setConfirm] = useState<PendingConfirm | null>(null);
  const store = useRef<ChatStore | null>(null);
  const connection = useRef<ServerConnection | null>(null);
  const list = useRef<FlatList<ChatMessage>>(null);

  useEffect(() => {
    let active = true;
    let unsubscribe = () => {};
    let unsubscribeConfirm = () => {};
    loadPairedDevice()
      .then((device) => {
        if (!active) return;
        if (!device) {
          router.replace('/pair');
          return;
        }
        const client = getServerConnection(device);
        connection.current = client;
        store.current = new ChatStore(client, {
          onMessages: setMessages,
          onSending: setSending,
          onError: setError,
        });
        // 高风险工具的确认请求会广播到所有设备，聊天页负责把它变成一次交互。
        unsubscribeConfirm = client.subscribe((envelope) => {
          if (envelope.type !== 'tool.confirm') return;
          const payload = envelope.payload as unknown as ToolConfirmPayload;
          setConfirm({
            callId: payload.call_id,
            tool: payload.tool,
            summary: payload.summary,
            expiresIn: payload.expires_in_ms,
          });
        });
        // 连接方式只能在回到状态回调里读：ref 在渲染期间不可访问。
        unsubscribe = client.onStatus((next) => {
          setStatus(next);
          setNetwork(client.network);
        });
        client.connect();
      })
      .catch((reason) => setError(String(reason)));
    return () => {
      active = false;
      unsubscribe();
      unsubscribeConfirm();
      store.current?.dispose();
    };
  }, []);

  // 超时后自动收起，避免用户面对一个早已失效的确认框。
  useEffect(() => {
    if (!confirm) return;
    const timer = setTimeout(() => setConfirm(null), Math.max(1000, confirm.expiresIn));
    return () => clearTimeout(timer);
  }, [confirm]);

  const decide = (approve: boolean) => {
    if (confirm) {
      connection.current?.send('tool.decision', { call_id: confirm.callId, approve });
    }
    setConfirm(null);
  };

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
            {connected
              ? network === 'public'
                ? '已连接（外网）'
                : '已连接（局域网）'
              : status === 'connecting'
                ? '正在连接电脑…'
                : '离线，正在重连'}
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

      <Modal
        transparent
        visible={confirm !== null}
        animationType="fade"
        onRequestClose={() => decide(false)}
      >
        <View style={styles.backdrop}>
          <View style={styles.confirmCard}>
            <Text style={styles.confirmTitle}>Pom 想执行一个操作</Text>
            <Text style={styles.confirmSummary}>{confirm?.summary}</Text>
            <Text style={styles.confirmHint}>
              工具 {confirm?.tool} · {Math.round((confirm?.expiresIn ?? 0) / 1000)} 秒内不回应视为拒绝
            </Text>
            <View style={styles.confirmRow}>
              <Pressable style={[styles.confirmButton, styles.reject]} onPress={() => decide(false)}>
                <Text style={styles.rejectText}>拒绝</Text>
              </Pressable>
              <Pressable style={[styles.confirmButton, styles.approve]} onPress={() => decide(true)}>
                <Text style={styles.approveText}>允许</Text>
              </Pressable>
            </View>
          </View>
        </View>
      </Modal>
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
  backdrop: {
    flex: 1,
    backgroundColor: '#23362f66',
    alignItems: 'center',
    justifyContent: 'center',
    padding: 24,
  },
  confirmCard: { backgroundColor: '#fffdf8', borderRadius: 22, padding: 22, width: '100%' },
  confirmTitle: { color: '#23362f', fontSize: 17, fontWeight: '800' },
  confirmSummary: { color: '#3c4a43', fontSize: 15, lineHeight: 23, marginTop: 12 },
  confirmHint: { color: '#87918b', fontSize: 12, marginTop: 10 },
  confirmRow: { flexDirection: 'row', gap: 12, marginTop: 20 },
  confirmButton: { flex: 1, paddingVertical: 13, borderRadius: 14, alignItems: 'center' },
  reject: { backgroundColor: '#f0ece1' },
  rejectText: { color: '#7c5c53', fontWeight: '700' },
  approve: { backgroundColor: '#547665' },
  approveText: { color: '#fffdf8', fontWeight: '700' },
});
