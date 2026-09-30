import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Play, Pause } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AudioSource } from "@/lib/historyAttachment";

interface AudioPlayerProps {
  /** Audio source URL. If not provided, onLoadRequest must be provided. */
  src?: string;
  /** 每次开始/恢复播放重新获取租约；signal 取消未完成的获取。 */
  onLoadRequest?: (signal: AbortSignal) => Promise<AudioSource | null>;
  className?: string;
  autoPlay?: boolean;
}

interface AudioPlayerGroupContextValue {
  requestPlayback: (audio: HTMLAudioElement) => void;
  releasePlayback: (audio: HTMLAudioElement) => void;
}

const AudioPlayerGroupContext =
  createContext<AudioPlayerGroupContextValue | null>(null);

export const AudioPlayerGroup: React.FC<React.PropsWithChildren> = ({
  children,
}) => {
  const activeAudioRef = useRef<HTMLAudioElement | null>(null);
  const value = useMemo<AudioPlayerGroupContextValue>(
    () => ({
      requestPlayback: (audio) => {
        if (activeAudioRef.current !== audio) activeAudioRef.current?.pause();
        activeAudioRef.current = audio;
      },
      releasePlayback: (audio) => {
        if (activeAudioRef.current === audio) activeAudioRef.current = null;
      },
    }),
    [],
  );

  return (
    <AudioPlayerGroupContext.Provider value={value}>
      {children}
    </AudioPlayerGroupContext.Provider>
  );
};

export const AudioPlayer: React.FC<AudioPlayerProps> = ({
  src: initialSrc,
  onLoadRequest,
  className = "",
  autoPlay = false,
}) => {
  const group = useContext(AudioPlayerGroupContext);
  const { t } = useTranslation();
  const [playbackError, setPlaybackError] = useState(false);
  const [isPlaying, setIsPlaying] = useState(false);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const [loadedSrc, setLoadedSrc] = useState<string | null>(initialSrc ?? null);
  const [isLoading, setIsLoading] = useState(false);

  const audioRef = useRef<HTMLAudioElement>(null);
  const sourceRef = useRef<AudioSource | null>(null);
  const requestRef = useRef<AbortController | null>(null);
  const mountedRef = useRef(false);
  const generationRef = useRef(0);
  const resumeTimeRef = useRef(0);
  const busyRef = useRef(false);
  const animationRef = useRef<number>();
  const dragTimeRef = useRef<number>(0);

  // Use refs to avoid stale closures in animation loop
  const isPlayingRef = useRef(false);
  const isDraggingRef = useRef(false);

  // Keep refs in sync with state
  useEffect(() => {
    isPlayingRef.current = isPlaying;
  }, [isPlaying]);

  useEffect(() => {
    isDraggingRef.current = isDragging;
  }, [isDragging]);

  // Stable animation loop with no dependencies
  const tick = useCallback(() => {
    if (audioRef.current && !isDraggingRef.current) {
      const time = audioRef.current.currentTime;
      setCurrentTime(time);
    }

    if (isPlayingRef.current) {
      animationRef.current = requestAnimationFrame(tick);
    }
  }, []); // Empty dependency array is key!

  // Manage animation loop lifecycle
  useEffect(() => {
    if (isPlaying && !isDragging) {
      // Only start if not already running
      if (!animationRef.current) {
        animationRef.current = requestAnimationFrame(tick);
      }
    } else {
      // Stop animation loop
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
        animationRef.current = undefined;
      }
    }

    return () => {
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
        animationRef.current = undefined;
      }
    };
  }, [isPlaying, isDragging, tick]);

  const stopPlayback = useCallback(
    (resetPosition = false, audio = audioRef.current) => {
      generationRef.current += 1;
      busyRef.current = false;
      const pending = requestRef.current;
      requestRef.current = null;
      const source = sourceRef.current;
      sourceRef.current = null;
      isPlayingRef.current = false;
      if (audio) {
        if (Number.isFinite(audio.currentTime))
          resumeTimeRef.current = audio.currentTime;
        if (resetPosition) resumeTimeRef.current = 0;
        // 先移除底层读取来源再释放租约。
        audio.pause();
        if (source) {
          audio.removeAttribute("src");
          audio.load();
        }
        group?.releasePlayback(audio);
      }
      source?.release();
      pending?.abort();
      if (mountedRef.current) {
        setIsPlaying(false);
        setIsLoading(false);
        if (source) setLoadedSrc(null);
      }
    },
    [group],
  );

  useEffect(() => {
    mountedRef.current = true;
    // React 在被动卸载清理前清空 DOM ref；保留实例以先停止文件读取再释放租约。
    const audio = audioRef.current;
    return () => {
      mountedRef.current = false;
      stopPlayback(false, audio);
    };
  }, [stopPlayback]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    const metadata = () => {
      setDuration(audio.duration || 0);
      const position = Math.min(resumeTimeRef.current, audio.duration || 0);
      if (position > 0) audio.currentTime = position;
      setCurrentTime(position);
    };
    const ended = () => {
      stopPlayback(true);
      setCurrentTime(0);
    };
    const play = () => {
      group?.requestPlayback(audio);
      isPlayingRef.current = true;
      setIsPlaying(true);
    };
    const pause = () => {
      // stopPlayback 清空引用后可能触发 pause；该事件不重复清理。
      if (sourceRef.current || requestRef.current || isPlayingRef.current)
        stopPlayback();
    };
    const error = () => {
      if (!sourceRef.current && !requestRef.current && !initialSrc) return;
      setPlaybackError(true);
      stopPlayback();
    };
    audio.addEventListener("loadedmetadata", metadata);
    audio.addEventListener("ended", ended);
    audio.addEventListener("play", play);
    audio.addEventListener("pause", pause);
    audio.addEventListener("error", error);
    return () => {
      audio.removeEventListener("loadedmetadata", metadata);
      audio.removeEventListener("ended", ended);
      audio.removeEventListener("play", play);
      audio.removeEventListener("pause", pause);
      audio.removeEventListener("error", error);
    };
  }, [group, initialSrc, stopPlayback]);

  // Global drag handlers
  const handleMouseUp = useCallback(() => {
    if (isDragging) {
      setIsDragging(false);
      if (audioRef.current) {
        if (loadedSrc) audioRef.current.currentTime = dragTimeRef.current;
        resumeTimeRef.current = dragTimeRef.current;
        setCurrentTime(dragTimeRef.current);
      }
    }
  }, [isDragging, loadedSrc]);

  useEffect(() => {
    if (isDragging) {
      document.addEventListener("mouseup", handleMouseUp);
      document.addEventListener("touchend", handleMouseUp);

      return () => {
        document.removeEventListener("mouseup", handleMouseUp);
        document.removeEventListener("touchend", handleMouseUp);
      };
    }
  }, [isDragging, handleMouseUp]);

  const startPlayback = useCallback(async () => {
    const audio = audioRef.current;
    if (!audio || busyRef.current) return;
    busyRef.current = true;
    const generation = ++generationRef.current;
    const request = new AbortController();
    requestRef.current = request;
    setPlaybackError(false);
    setIsLoading(true);
    try {
      if (onLoadRequest) {
        const source = await onLoadRequest(request.signal);
        if (
          !mountedRef.current ||
          request.signal.aborted ||
          generation !== generationRef.current
        ) {
          source?.release();
          return;
        }
        if (!source) {
          stopPlayback();
          return;
        }
        sourceRef.current = source;
        audio.src = source.url;
        setLoadedSrc(source.url);
      } else if (initialSrc) {
        audio.src = initialSrc;
      } else {
        stopPlayback();
        return;
      }
      await audio.play();
      if (!mountedRef.current || generation !== generationRef.current) return;
      setIsLoading(false);
      busyRef.current = false;
    } catch {
      if (mountedRef.current && generation === generationRef.current) {
        setPlaybackError(true);
        stopPlayback();
      }
    }
  }, [initialSrc, onLoadRequest, stopPlayback]);

  useEffect(() => {
    if (autoPlay && initialSrc) void startPlayback();
  }, [autoPlay, initialSrc, startPlayback]);

  const togglePlay = () => {
    if (busyRef.current || isPlayingRef.current) stopPlayback();
    else void startPlayback();
  };

  const handleSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newTime = parseFloat(e.target.value);
    dragTimeRef.current = newTime;
    setCurrentTime(newTime);

    if (!isDragging && audioRef.current) {
      if (loadedSrc) audioRef.current.currentTime = newTime;
      resumeTimeRef.current = newTime;
    }
  };

  const handleSliderMouseDown = () => {
    setIsDragging(true);
  };

  const handleSliderTouchStart = () => {
    setIsDragging(true);
  };

  const formatTime = (time: number): string => {
    if (!isFinite(time)) return "0:00";

    const minutes = Math.floor(time / 60);
    const seconds = Math.floor(time % 60);
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  };

  // Fix playhead positioning with better edge case handling
  const getProgressPercent = (): number => {
    if (duration <= 0) return 0;

    // Handle the end case - if we're within 0.1 seconds of the end, show 100%
    if (duration - currentTime < 0.1) return 100;

    const percent = (currentTime / duration) * 100;
    return Math.min(100, Math.max(0, percent));
  };

  const progressPercent = getProgressPercent();

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <audio ref={audioRef} preload="none" />

      <button
        onClick={togglePlay}
        aria-busy={isLoading}
        className="transition-colors cursor-pointer text-text hover:text-accent-text disabled:opacity-50"
        aria-label={
          isPlaying || isLoading
            ? t("settings.history.audio.pause")
            : t("settings.history.audio.play")
        }
      >
        {isPlaying ? (
          <Pause width={20} height={20} fill="currentColor" />
        ) : (
          <Play width={20} height={20} fill="currentColor" />
        )}
      </button>

      {playbackError && (
        <span role="alert" className="text-xs text-text/60">
          {t("settings.history.audio.failed")}
        </span>
      )}

      <div className="flex-1 flex items-center gap-2">
        <span className="text-xs text-text/60 min-w-[30px] tabular-nums">
          {formatTime(currentTime)}
        </span>

        <input
          type="range"
          min="0"
          max={duration || 0}
          step="0.01"
          value={currentTime}
          onChange={handleSeek}
          onMouseDown={handleSliderMouseDown}
          onTouchStart={handleSliderTouchStart}
          className={`flex-1 h-1 rounded-lg appearance-none cursor-pointer focus:outline-none focus:ring-1 focus:ring-logo-primary ${progressPercent >= 99.5 ? "[&::-webkit-slider-thumb]:translate-x-0.5 [&::-moz-range-thumb]:translate-x-0.5" : ""}`}
          style={{
            background: `linear-gradient(to right, #FAA2CA 0%, #FAA2CA ${progressPercent}%, rgba(128, 128, 128, 0.2) ${progressPercent}%, rgba(128, 128, 128, 0.2) 100%)`,
          }}
        />

        <span className="text-xs text-text/60 min-w-[30px] tabular-nums">
          {formatTime(duration)}
        </span>
      </div>
    </div>
  );
};
