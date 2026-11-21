import { Redirect } from 'expo-router';
import { useEffect, useState } from 'react';
import { View } from 'react-native';

import { Pom } from '../components/Pom';
import { loadPairedDevice } from '../lib/identity';

export default function Index() {
  const [paired, setPaired] = useState<boolean | null>(null);
  useEffect(() => {
    loadPairedDevice().then((device) => setPaired(device !== null)).catch(() => setPaired(false));
  }, []);
  if (paired === null) {
    return <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}><Pom /></View>;
  }
  return <Redirect href={paired ? '/chat' : '/pair'} />;
}
