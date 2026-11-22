import { router } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useMemo, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { getServerConnection, type ConnectionStatus, type ServerConnection } from '../lib/connection';
import { loadPairedDevice } from '../lib/identity';
import {
  cancelVisionTask,
  createVisionTask,
  reduceVisionEnvelope,
  startVisionTask,
  type VisionTaskState,
} from '../lib/vision';

const STATUS_LABELS: Record<VisionTaskState['status'], string> = {
  idle: '准备就绪',
  running: '执行中',
  started: '任务已启动',
  observed: '正在观察窗口',
  actions: '正在执行动作',
  finished: '已完成',
  cancelled: '已取消',
  done: '已完成',
  failed: '执行失败',
};

export default function VisionScreen() {
  const [goal, setGoal] = useState('');
  const [windowTitle, setWindowTitle] = useState('');
  const [task, setTask] = useState<VisionTaskState | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('connecting');
  const [error, setError] = useState('');
  const connection = useRef<ServerConnection | null>(null);
  const taskRef = useRef<VisionTaskState | null>(null);
  const taskActive = task?.status === 'started' || task?.status === 'observed' || task?.status === 'actions' || task?.status === 'running';
  const connected = connectionStatus === 'connected';

  useEffect(() => {
    let active = true;
    let unsubscribe = () => {};
    let unsubscribeStatus = () => {};
    loadPairedDevice()
      .then((device) => {
        if (!active) return;
        if (!device) {
          router.replace('/pair');
          return;
        }
        const client = getServerConnection(device);
        connection.current = client;
        unsubscribeStatus = client.onStatus(setConnectionStatus);
        unsubscribe = client.subscribe((envelope) => {
          const current = taskRef.current;
          if (!current) return;
          if (envelope.type === 'error') {
            const payload = envelope.payload as { message?: string };
            setError(payload.message ?? '电脑端返回了错误');
            return;
          }
          const next = reduceVisionEnvelope(current, envelope);
          if (next !== current) {
            taskRef.current = next;
            setTask(next);
          }
        });
        client.connect();
      })
      .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
    return () => {
      active = false;
      unsubscribe();
      unsubscribeStatus();
    };
  }, []);

  const statusText = useMemo(() => {
    if (!task) return '填写目标后，让 Pom 操作电脑上的窗口。';
    return STATUS_LABELS[task.status];
  }, [task]);

  const start = () => {
    const trimmedGoal = goal.trim();
    const trimmedWindow = windowTitle.trim();
    if (!trimmedGoal || !trimmedWindow || !connection.current || !connected) return;
    setError('');
    const next = createVisionTask(`${Date.now()}-${Math.random().toString(36).slice(2, 8)}`, trimmedGoal, trimmedWindow);
    next.status = 'running';
    taskRef.current = next;
    setTask(next);
    if (!startVisionTask(connection.current, next)) {
      taskRef.current = { ...next, status: 'failed', success: false, message: '当前连接不可用' };
      setTask(taskRef.current);
    }
  };

  const cancel = () => {
    if (!taskRef.current || !connection.current) return;
    cancelVisionTask(connection.current, taskRef.current.taskId);
    const next = { ...taskRef.current, status: 'cancelled' as const, success: false, message: '正在请求取消…' };
    taskRef.current = next;
    setTask(next);
  };

  return (
    <SafeAreaView style={styles.safe}>
      <StatusBar style="dark" />
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <View style={styles.topline}>
          <Pressable onPress={() => router.back()} hitSlop={12}><Text style={styles.back}>‹ 返回</Text></Pressable>
          <Text style={styles.eyebrow}>DESKTOP VISION</Text>
        </View>
        <Text style={styles.title}>让 Pom 看着做</Text>
        <Text style={styles.subtitle}>把目标交给电脑端的视觉 Agent。它会截图、观察窗口、执行动作，并把每一步回传到这里。</Text>

        {error ? <Pressable style={styles.error} onPress={() => setError('')}><Text style={styles.errorText}>{error}</Text></Pressable> : null}

        <View style={styles.card}>
          <Text style={styles.label}>目标窗口标题</Text>
          <TextInput
            value={windowTitle}
            onChangeText={setWindowTitle}
            placeholder="例如：Chrome、记事本"
            placeholderTextColor="#9aa29d"
            style={styles.input}
            editable={!taskActive}
          />
          <Text style={styles.label}>你希望它完成什么？</Text>
          <TextInput
            value={goal}
            onChangeText={setGoal}
            placeholder="例如：打开百度，搜索天气并告诉我今天是否需要带伞"
            placeholderTextColor="#9aa29d"
            style={[styles.input, styles.goalInput]}
            multiline
            maxLength={32000}
            editable={!taskActive}
          />
          <Pressable style={[styles.start, (!goal.trim() || !windowTitle.trim() || !connected || taskActive) && styles.disabled]} onPress={start} disabled={!goal.trim() || !windowTitle.trim() || !connected || taskActive}>
            {!connected ? <ActivityIndicator color="#fffdf8" /> : <Text style={styles.startText}>开始视觉任务</Text>}
          </Pressable>
          <Text style={styles.connection}>{connected ? '电脑已连接' : connectionStatus === 'connecting' ? '正在连接电脑…' : '电脑离线，暂时不能开始'}</Text>
        </View>

        {task ? (
          <View style={styles.progressCard}>
            <View style={styles.progressHead}>
              <View><Text style={styles.sectionLabel}>LIVE RUN</Text><Text style={styles.status}>{statusText}</Text></View>
              {taskActive ? <Pressable style={styles.cancel} onPress={cancel}><Text style={styles.cancelText}>中止</Text></Pressable> : null}
            </View>
            <View style={styles.metrics}>
              <View><Text style={styles.metricValue}>{task.step}</Text><Text style={styles.metricLabel}>步骤</Text></View>
              <View><Text style={styles.metricValue}>{task.actionCount}</Text><Text style={styles.metricLabel}>动作</Text></View>
              <View style={styles.metricWide}><Text style={styles.metricLabel}>窗口</Text><Text style={styles.metricWindow} numberOfLines={1}>{task.windowTitle}</Text></View>
            </View>
            <Text style={styles.message}>{task.message || '等待电脑端响应…'}</Text>
            {!taskActive && task.status !== 'done' && task.status !== 'finished' ? <Pressable onPress={() => { taskRef.current = null; setTask(null); }}><Text style={styles.dismiss}>清除这次任务</Text></Pressable> : null}
          </View>
        ) : null}
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f5f1e8' },
  content: { padding: 22, paddingBottom: 44 },
  topline: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', marginBottom: 28 },
  back: { color: '#547665', fontSize: 15, fontWeight: '700' },
  eyebrow: { color: '#9a685e', fontSize: 11, fontWeight: '800', letterSpacing: 2 },
  title: { color: '#23362f', fontSize: 34, fontWeight: '800', letterSpacing: -1, maxWidth: 290 },
  subtitle: { color: '#65736c', fontSize: 15, lineHeight: 23, marginTop: 12, marginBottom: 24 },
  card: { backgroundColor: '#fffdf8', borderRadius: 24, padding: 18, shadowColor: '#23362f', shadowOpacity: 0.06, shadowRadius: 18, shadowOffset: { width: 0, height: 8 }, elevation: 2 },
  label: { color: '#3c4a43', fontSize: 13, fontWeight: '800', marginBottom: 8, marginTop: 4 },
  input: { backgroundColor: '#f5f1e8', color: '#23362f', borderRadius: 14, paddingHorizontal: 14, paddingVertical: 12, fontSize: 15, marginBottom: 16 },
  goalInput: { minHeight: 112, textAlignVertical: 'top' },
  start: { backgroundColor: '#547665', borderRadius: 15, minHeight: 50, alignItems: 'center', justifyContent: 'center' },
  startText: { color: '#fffdf8', fontSize: 15, fontWeight: '800' },
  disabled: { opacity: 0.45 },
  connection: { color: '#87918b', textAlign: 'center', fontSize: 12, marginTop: 10 },
  error: { backgroundColor: '#f7e5df', borderRadius: 14, padding: 12, marginBottom: 16 },
  errorText: { color: '#834d43', lineHeight: 20 },
  progressCard: { backgroundColor: '#23362f', borderRadius: 24, padding: 20, marginTop: 18 },
  progressHead: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'flex-start' },
  sectionLabel: { color: '#a9bba7', fontSize: 10, fontWeight: '800', letterSpacing: 2 },
  status: { color: '#fffdf8', fontSize: 22, fontWeight: '800', marginTop: 5 },
  cancel: { borderColor: '#d99b8e', borderWidth: 1, borderRadius: 12, paddingHorizontal: 13, paddingVertical: 8 },
  cancelText: { color: '#f0c1b6', fontWeight: '800' },
  metrics: { flexDirection: 'row', gap: 24, borderTopColor: '#ffffff1c', borderTopWidth: 1, borderBottomColor: '#ffffff1c', borderBottomWidth: 1, paddingVertical: 16, marginTop: 20 },
  metricValue: { color: '#d9e8d5', fontSize: 24, fontWeight: '800' },
  metricLabel: { color: '#a9bba7', fontSize: 11, marginTop: 2 },
  metricWide: { flex: 1, justifyContent: 'center' },
  metricWindow: { color: '#fffdf8', fontSize: 14, marginTop: 4 },
  message: { color: '#e4ede1', fontSize: 15, lineHeight: 23, marginTop: 18 },
  dismiss: { color: '#a9bba7', fontSize: 13, marginTop: 18 },
});
