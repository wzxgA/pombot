import { CameraView, useCameraPermissions } from 'expo-camera';
import { router } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useZeroconf } from 'react-native-zeroconf';

import { Pom } from '../components/Pom';
import { parsePairTicket, type PairTicket } from '../lib/identity';
import { beginPairing, confirmPairing, type PairSession } from '../lib/pairing';

interface FoundComputer {
  name: string;
  ticket: PairTicket;
}

const DISCOVERY_OPTIONS = { type: 'pombot', protocol: 'tcp', domain: 'local.' };

export default function PairScreen() {
  const [permission, requestPermission] = useCameraPermissions();
  const [session, setSession] = useState<PairSession | null>(null);
  const [selected, setSelected] = useState<FoundComputer | null>(null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const starting = useRef(false);
  const sessionRef = useRef<PairSession | null>(null);
  const { services } = useZeroconf(DISCOVERY_OPTIONS);
  const found: FoundComputer[] = services.flatMap((service) => {
    const address = service.ipv4[0] ?? service.addresses[0] ?? service.host;
    return address ? [{
      name: service.name || 'PomBot 电脑',
      ticket: { serverUrl: `http://${address}:${service.port}`, code: '', serverPublicKey: '' },
    }] : [];
  });

  useEffect(() => () => sessionRef.current?.socket.close(), []);

  const start = async (ticket: PairTicket) => {
    if (starting.current) return;
    starting.current = true;
    setBusy(true);
    setError('');
    try {
      const next = await beginPairing(ticket);
      sessionRef.current = next;
      setSession(next);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      starting.current = false;
      setBusy(false);
    }
  };

  const scan = ({ data }: { data: string }) => {
    if (busy || session) return;
    try {
      void start(parsePairTicket(data));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };

  const confirm = async () => {
    if (!session) return;
    setBusy(true);
    setError('');
    try {
      await confirmPairing(session);
      router.replace('/chat');
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  const cancel = () => {
    session?.socket.close();
    sessionRef.current = null;
    setSession(null);
    setSelected(null);
    setCode('');
  };

  return (
    <SafeAreaView style={styles.safe}>
      <StatusBar style="dark" />
      {session ? (
        <View style={styles.center}>
          <Pom size={124} />
          <Text style={styles.title}>核对短词</Text>
          <Text style={styles.description}>请确认电脑配对页面显示了相同的四个词</Text>
          <View style={styles.wordCard}><Text style={styles.word}>{session.shortWord}</Text></View>
          {error ? <Text style={styles.error}>{error}</Text> : null}
          <Pressable style={styles.primary} onPress={confirm} disabled={busy}>
            {busy ? <ActivityIndicator color="#fffdf8" /> : <Text style={styles.primaryText}>一致，完成配对</Text>}
          </Pressable>
          <Pressable onPress={cancel}><Text style={styles.secondary}>不一致，取消</Text></Pressable>
        </View>
      ) : selected ? (
        <View style={styles.center}>
          <Pom size={110} />
          <Text style={styles.title}>连接 {selected.name}</Text>
          <Text style={styles.description}>输入电脑配对页面上的 6 位数字</Text>
          <TextInput style={styles.code} value={code} onChangeText={setCode} keyboardType="number-pad" maxLength={6} placeholder="000000" autoFocus />
          {error ? <Text style={styles.error}>{error}</Text> : null}
          <Pressable style={[styles.primary, (!/^\d{6}$/.test(code) || busy) && styles.disabled]} disabled={!/^\d{6}$/.test(code) || busy} onPress={() => start({ ...selected.ticket, code })}>
            {busy ? <ActivityIndicator color="#fffdf8" /> : <Text style={styles.primaryText}>继续</Text>}
          </Pressable>
          <Pressable onPress={cancel}><Text style={styles.secondary}>返回扫码</Text></Pressable>
        </View>
      ) : (
        <View style={styles.container}>
          <Text style={styles.heading}>连接你的电脑</Text>
          <Text style={styles.subheading}>扫描电脑 PomBot 配对页面上的二维码</Text>
          <View style={styles.cameraFrame}>
            {permission?.granted ? <CameraView style={StyleSheet.absoluteFill} facing="back" barcodeScannerSettings={{ barcodeTypes: ['qr'] }} onBarcodeScanned={scan} /> : <View style={styles.permission}><Pom size={88} /><Text style={styles.description}>需要相机权限来扫码</Text><Pressable style={styles.primary} onPress={requestPermission}><Text style={styles.primaryText}>允许使用相机</Text></Pressable></View>}
            {busy ? <View style={styles.loading}><ActivityIndicator size="large" color="#fffdf8" /></View> : null}
          </View>
          {error ? <Text style={styles.error}>{error}</Text> : null}
          {found.length ? <Text style={styles.foundTitle}>同一 WiFi 下的电脑</Text> : <Text style={styles.discovering}>正在自动发现电脑…</Text>}
          {found.map((computer) => <Pressable key={computer.ticket.serverUrl} style={styles.computer} onPress={() => { setError(''); setSelected(computer); }}><Text style={styles.computerName}>{computer.name}</Text><Text style={styles.computerAction}>输入配对码</Text></Pressable>)}
        </View>
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f5f1e8' },
  container: { flex: 1, padding: 24 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: 28 },
  heading: { color: '#23362f', fontSize: 28, fontWeight: '800', marginTop: 14 },
  subheading: { color: '#65736c', fontSize: 15, marginTop: 8, marginBottom: 24 },
  title: { color: '#23362f', fontSize: 26, fontWeight: '800', marginTop: 22, textAlign: 'center' },
  description: { color: '#65736c', fontSize: 15, lineHeight: 23, textAlign: 'center', marginTop: 10, marginBottom: 20 },
  cameraFrame: { height: 340, borderRadius: 28, overflow: 'hidden', backgroundColor: '#dce4dc' },
  permission: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: 24 },
  loading: { position: 'absolute', top: 0, right: 0, bottom: 0, left: 0, backgroundColor: '#23362f66', alignItems: 'center', justifyContent: 'center' },
  primary: { minWidth: 210, minHeight: 50, borderRadius: 16, backgroundColor: '#547665', alignItems: 'center', justifyContent: 'center', paddingHorizontal: 24, marginTop: 12 },
  primaryText: { color: '#fffdf8', fontWeight: '700', fontSize: 16 },
  secondary: { color: '#65736c', padding: 18 },
  disabled: { opacity: 0.45 },
  error: { color: '#934f43', lineHeight: 21, textAlign: 'center', marginVertical: 12 },
  wordCard: { backgroundColor: '#fffdf8', borderRadius: 20, padding: 24, marginVertical: 12 },
  word: { color: '#547665', fontSize: 22, fontWeight: '800', textAlign: 'center' },
  code: { backgroundColor: '#fffdf8', borderRadius: 18, fontSize: 32, letterSpacing: 12, textAlign: 'center', width: 250, padding: 15, color: '#23362f', marginBottom: 12 },
  discovering: { color: '#65736c', textAlign: 'center', marginTop: 24 },
  foundTitle: { color: '#23362f', fontWeight: '700', marginTop: 24, marginBottom: 8 },
  computer: { backgroundColor: '#fffdf8', borderRadius: 14, padding: 16, flexDirection: 'row', justifyContent: 'space-between' },
  computerName: { color: '#23362f', fontWeight: '600' },
  computerAction: { color: '#547665' },
});
