import type {
  Envelope,
  VisionDonePayload,
  VisionProgressPayload,
} from '../generated/contract';
import type { ServerConnection } from './connection';

export type VisionTaskStatus = VisionProgressPayload['status'] | 'idle' | 'running' | 'done' | 'failed';

export interface VisionTaskState {
  taskId: string;
  goal: string;
  windowTitle: string;
  status: VisionTaskStatus;
  step: number;
  actionCount: number;
  message: string;
  success: boolean | null;
}

export function createVisionTask(taskId: string, goal: string, windowTitle: string): VisionTaskState {
  return {
    taskId,
    goal,
    windowTitle,
    status: 'idle',
    step: 0,
    actionCount: 0,
    message: '',
    success: null,
  };
}

export function startVisionTask(connection: ServerConnection, task: VisionTaskState): boolean {
  return connection.send('vision.start', {
    task_id: task.taskId,
    goal: task.goal,
    window_title: task.windowTitle,
  });
}

export function cancelVisionTask(connection: ServerConnection, taskId: string): boolean {
  return connection.send('vision.cancel', { task_id: taskId });
}

export function reduceVisionEnvelope(state: VisionTaskState, envelope: Envelope): VisionTaskState {
  if (envelope.type === 'vision.progress') {
    const payload = envelope.payload as VisionProgressPayload;
    if (payload.task_id !== state.taskId) return state;
    return {
      ...state,
      status: payload.status,
      step: payload.step,
      actionCount: payload.action_count,
      message: payload.message,
      success: payload.status === 'finished' ? true : payload.status === 'failed' || payload.status === 'cancelled' ? false : state.success,
    };
  }
  if (envelope.type === 'vision.done') {
    const payload = envelope.payload as VisionDonePayload;
    if (payload.task_id !== state.taskId) return state;
    return {
      ...state,
      status: payload.success ? 'done' : 'failed',
      message: payload.message,
      success: payload.success,
    };
  }
  return state;
}
