import 'package:bloc/bloc.dart';
import 'package:equatable/equatable.dart';

import '../data/api_service.dart';
import '../data/models.dart';

/// Состояние процесса сканирования.
sealed class ScannerState extends Equatable {
  const ScannerState();

  @override
  List<Object?> get props => [];
}

/// Начальное состояние.
class ScannerInitial extends ScannerState {
  const ScannerInitial();
}

/// Идёт захват/обработка разворота.
class ScannerScanning extends ScannerState {
  const ScannerScanning();
}

/// Разворот обработан, вершины получены.
class ScannerSuccess extends ScannerState {
  final ScanResponse response;
  const ScannerSuccess(this.response);

  @override
  List<Object?> get props => [response];
}

/// Ошибка (сервер недоступен, ошибка обработки).
class ScannerError extends ScannerState {
  final String message;
  const ScannerError(this.message);

  @override
  List<Object?> get props => [message];
}

/// События BLoC.
sealed class ScannerEvent extends Equatable {
  const ScannerEvent();

  @override
  List<Object?> get props => [];
}

/// Запустить захват + обработку разворота.
class StartScan extends ScannerEvent {
  final ScanProfile profile;
  const StartScan({this.profile = ScanProfile.textBw1bit});

  @override
  List<Object?> get props => [profile];
}

/// Сбросить состояние.
class ResetScan extends ScannerEvent {
  const ResetScan();
}

/// BLoC процесса сканирования.
class ScannerBloc extends Bloc<ScannerEvent, ScannerState> {
  final ApiService _api;

  ScannerBloc(this._api) : super(ScannerInitial()) {
    on<StartScan>(_onStartScan);
    on<ResetScan>(_onReset);
  }

  Future<void> _onStartScan(
    StartScan event,
    Emitter<ScannerState> emit,
  ) async {
    emit(ScannerScanning());
    try {
      // Инициализация каретки (если ещё не инициализирована)
      await _api.initScanner();

      // UUID сессии — простой уникальный идентификатор
      final uuid = DateTime.now().microsecondsSinceEpoch.toRadixString(16);

      final response = await _api.processScan(
        uuid: uuid,
        profile: event.profile,
      );
      emit(ScannerSuccess(response));
    } on ApiException catch (e) {
      emit(ScannerError(e.message));
    } catch (e) {
      emit(ScannerError('Сервер недоступен: $e'));
    }
  }

  void _onReset(ResetScan event, Emitter<ScannerState> emit) {
    emit(ScannerInitial());
  }
}