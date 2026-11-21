// @author wzxgqtl 2026-11-21
// 笔记编辑器。
//
// 进入即取锁：拿到才是可写，拿不到就是只读并显示"谁在编辑"。
// 这样不存在两端同时改同一篇的可能，同步层也就不需要做合并。

import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { getServerConnection } from '../lib/connection';
import { loadPairedDevice } from '../lib/identity';
import { NotesStore, type ObjectItem } from '../lib/notes';

type LockPhase = 'acquiring' | 'editing' | 'readonly';

const HEARTBEAT_MS = 15_000;

export default function NoteScreen() {
  const params = useLocalSearchParams<{ id?: string }>();
  const objectId = params.id ?? '';

  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [item, setItem] = useState<ObjectItem | null>(null);
  const [phase, setPhase] = useState<LockPhase>(objectId ? 'acquiring' : 'editing');
  const [holder, setHolder] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);

  const store = useRef<NotesStore | null>(null);
  const lockedId = useRef<string>('');
  const dirty = useRef(false);

  useEffect(() => {
    let active = true;
    let unsubscribeStatus = () => {};
    let heartbeat: ReturnType<typeof setInterval> | null = null;

    loadPairedDevice()
      .then(async (device) => {
        if (!active) return;
        if (!device) {
          router.replace('/pair');
          return;
        }
        const connection = getServerConnection(device);
        const notes = new NotesStore(connection, {
          onChanged: () => {
            if (!active || !objectId) return;
            const latest = notes.byId(objectId);
            if (!latest) return;
            setItem(latest);
            // 未产生本地改动时跟随服务端，避免把旧内容覆盖回去。
            if (!dirty.current) {
              setTitle(latest.title);
              setContent(latest.content);
            }
          },
          onError: setError,
          onLockChanged: (changedId, info) => {
            if (!active || changedId !== objectId) return;
            if (info && lockedId.current !== objectId) {
              setHolder(info.holder);
              setPhase('readonly');
            }
          },
        });
        store.current = notes;
        unsubscribeStatus = connection.onStatus((status) => {
          if (status === 'connected') notes.pull();
        });
        connection.connect();

        if (!objectId) return;
        const result = await notes.acquireLock(objectId);
        if (!active) return;
        if (result.granted) {
          lockedId.current = objectId;
          setPhase('editing');
          heartbeat = setInterval(() => {
            if (!active) return;
            notes.heartbeat(objectId);
          }, HEARTBEAT_MS);
        } else {
          setHolder(result.holder);
          setPhase('readonly');
        }
      })
      .catch((reason) => setError(String(reason)));

    return () => {
      active = false;
      if (heartbeat) clearInterval(heartbeat);
      unsubscribeStatus();
      if (lockedId.current) store.current?.releaseLock(lockedId.current);
      store.current?.dispose();
    };
  }, [objectId]);

  const save = () => {
    const notes = store.current;
    if (!notes) return;
    setSaving(true);
    setError('');
    notes.save({
      id: objectId,
      kind: item?.kind ?? 'note',
      title: title.trim() || '未命名',
      content,
      baseRev: item?.rev ?? 0,
    });
    dirty.current = false;
    setSaving(false);
    if (!objectId) router.back();
  };

  const remove = () => {
    if (!objectId || !store.current) return;
    store.current.remove(objectId, item?.rev ?? 0);
    router.back();
  };

  const editable = phase === 'editing';

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.header}>
        <Pressable onPress={() => router.back()} hitSlop={12}>
          <Text style={styles.back}>返回</Text>
        </Pressable>
        <Text style={styles.heading}>{objectId ? '编辑' : '新建笔记'}</Text>
        <Pressable onPress={save} hitSlop={12} disabled={!editable || saving}>
          <Text style={[styles.save, !editable && styles.disabled]}>保存</Text>
        </Pressable>
      </View>

      {phase === 'acquiring' ? (
        <View style={styles.notice}>
          <ActivityIndicator color="#547665" />
          <Text style={styles.noticeText}>正在获取编辑权…</Text>
        </View>
      ) : null}

      {phase === 'readonly' ? (
        <View style={[styles.notice, styles.noticeWarn]}>
          <Text style={styles.noticeText}>✎ {holder || '其他设备'}正在编辑，此端为只读</Text>
        </View>
      ) : null}

      {error ? (
        <Pressable style={styles.error} onPress={() => setError('')}>
          <Text style={styles.errorText}>{error}</Text>
        </Pressable>
      ) : null}

      <ScrollView contentContainerStyle={styles.body} keyboardShouldPersistTaps="handled">
        <TextInput
          style={styles.titleInput}
          value={title}
          onChangeText={(value) => {
            dirty.current = true;
            setTitle(value);
          }}
          placeholder="标题"
          placeholderTextColor="#87918b"
          editable={editable}
          maxLength={200}
        />
        <TextInput
          style={styles.contentInput}
          value={content}
          onChangeText={(value) => {
            dirty.current = true;
            setContent(value);
          }}
          placeholder="写点什么…"
          placeholderTextColor="#87918b"
          editable={editable}
          multiline
          textAlignVertical="top"
          maxLength={200000}
        />
        {objectId ? (
          <Pressable style={styles.remove} onPress={remove} disabled={!editable}>
            <Text style={[styles.removeText, !editable && styles.disabled]}>删除这条</Text>
          </Pressable>
        ) : null}
      </ScrollView>
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
  heading: { color: '#23362f', fontSize: 18, fontWeight: '800' },
  save: { color: '#547665', fontSize: 15, fontWeight: '700' },
  disabled: { opacity: 0.4 },
  notice: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 10,
    marginHorizontal: 16,
    padding: 12,
    borderRadius: 12,
    backgroundColor: '#eef2ee',
  },
  noticeWarn: { backgroundColor: '#f7e5df' },
  noticeText: { color: '#4b5c53', fontSize: 13 },
  error: { backgroundColor: '#f7e5df', marginHorizontal: 16, marginTop: 8, padding: 12, borderRadius: 12 },
  errorText: { color: '#834d43', lineHeight: 20 },
  body: { padding: 20, gap: 12 },
  titleInput: { color: '#23362f', fontSize: 22, fontWeight: '800', paddingVertical: 6 },
  contentInput: { color: '#3c4a43', fontSize: 16, lineHeight: 25, minHeight: 320, paddingVertical: 6 },
  remove: { alignSelf: 'flex-start', marginTop: 24 },
  removeText: { color: '#a2675b', fontSize: 14 },
});
