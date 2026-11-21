import { View } from 'react-native';
import Svg, { Circle, Defs, Ellipse, RadialGradient, Stop } from 'react-native-svg';

interface PomProps {
  offline?: boolean;
  size?: number;
}

export function Pom({ offline = false, size = 112 }: PomProps) {
  const center = offline ? '#a7aaa5' : '#a9c9ad';
  const edge = offline ? '#686f6b' : '#547665';
  return (
    <View style={{ width: size, height: size }} accessibilityLabel={offline ? '离线绒球' : '绒球 Pom'}>
      <Svg width={size} height={size} viewBox="0 0 120 120">
        <Defs>
          <RadialGradient id="pom" cx="38%" cy="30%" r="70%">
            <Stop offset="0" stopColor="#fffdf8" />
            <Stop offset="0.45" stopColor={center} />
            <Stop offset="1" stopColor={edge} />
          </RadialGradient>
        </Defs>
        <Ellipse cx="60" cy="105" rx="31" ry="6" fill="#23362f" opacity="0.12" />
        <Circle cx="60" cy="58" r="43" fill="url(#pom)" />
        <Circle cx="46" cy="57" r="4" fill="#23362f" />
        <Circle cx="74" cy="57" r="4" fill="#23362f" />
        <Ellipse cx="60" cy="72" rx="7" ry={offline ? 2 : 5} fill="#23362f" opacity="0.75" />
      </Svg>
    </View>
  );
}
