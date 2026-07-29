/* Browser-backed persistence for the LÖVE save directory. */
(function (root) {
  "use strict";

  var WRITE_FLAGS = 2097155;

  function errorDetail(error) {
    if (error && error.message) {
      return String(error.message);
    }
    return String(error || "unknown storage error");
  }

  function create(options) {
    var fs = options.fs;
    var persistentRoot = options.persistent_root.replace(/\/+$/, "");
    var persistentPrefix = persistentRoot + "/";
    var schedule = options.schedule || root.setTimeout.bind(root);
    var onEvent = options.on_event || function () {};
    var onState = options.on_state || function () {};
    var forceUnavailable = options.force_unavailable === true;
    var originalSync = fs.syncfs.bind(fs);
    var originalClose = fs.close.bind(fs);
    var queue = [];
    var activeOperation = null;
    var flushScheduled = false;
    var attached = false;
    var state = {
      state: "initializing",
      populate_count: 0,
      flush_count: 0,
      skipped_flush_count: 0,
      last_operation: null,
      last_error: null
    };

    function snapshot() {
      return {
        state: state.state,
        populate_count: state.populate_count,
        flush_count: state.flush_count,
        skipped_flush_count: state.skipped_flush_count,
        last_operation: state.last_operation,
        last_error: state.last_error
      };
    }

    function publish() {
      onState(snapshot());
    }

    function emit(name, fields, level) {
      onEvent(name, fields, level || "info");
      publish();
    }

    function unavailable(operation, error) {
      state.state = "unavailable";
      state.last_operation = operation;
      state.last_error = {
        detail: errorDetail(error),
        operation: operation,
        recoverable: true
      };
      emit("storage_error", state.last_error, "warn");
    }

    function finish(request, error) {
      activeOperation = null;
      if (error) {
        unavailable(request.operation, error);
      } else {
        state.state = "ready";
        state.last_operation = request.operation;
        state.last_error = null;
        if (request.populate) {
          state.populate_count += 1;
        } else {
          state.flush_count += 1;
        }
        emit("storage_sync", {
          operation: request.operation,
          status: "ok"
        });
      }
      request.callback(null);
      runNext();
    }

    function runNext() {
      if (activeOperation || queue.length === 0) {
        return;
      }
      var request = queue.shift();
      if (forceUnavailable && state.state !== "unavailable") {
        unavailable(request.operation, "forced unavailable by browser diagnostics");
      }
      if (state.state === "unavailable") {
        if (!request.populate) {
          state.skipped_flush_count += 1;
          state.last_operation = request.operation;
          emit("storage_sync", {
            operation: request.operation,
            status: "skipped",
            recoverable: true
          });
        }
        request.callback(null);
        runNext();
        return;
      }

      activeOperation = request.operation;
      try {
        originalSync(request.populate, function (error) {
          finish(request, error);
        });
      } catch (error) {
        finish(request, error);
      }
    }

    function sync(populate, callback) {
      if (typeof populate === "function") {
        callback = populate;
        populate = false;
      }
      queue.push({
        callback: callback || function () {},
        operation: populate ? "populate" : "flush",
        populate: Boolean(populate)
      });
      runNext();
    }

    function scheduleFlush() {
      if (flushScheduled) {
        return;
      }
      flushScheduled = true;
      schedule(function () {
        flushScheduled = false;
        sync(false, function () {});
      }, 0);
    }

    function close(stream) {
      var path = stream && stream.path;
      var writable =
        stream &&
        typeof stream.flags === "number" &&
        (stream.flags & WRITE_FLAGS) !== 0;
      var persistent =
        typeof path === "string" &&
        (path === persistentRoot || path.indexOf(persistentPrefix) === 0);
      var result = originalClose(stream);
      if (writable && persistent && activeOperation !== "populate") {
        scheduleFlush();
      }
      return result;
    }

    function attach() {
      if (attached) {
        return;
      }
      attached = true;
      fs.syncfs = sync;
      fs.close = close;
      publish();
    }

    return {
      attach: attach,
      snapshot: snapshot
    };
  }

  root.GoliseoBrowserStorage = {
    create: create
  };
})(window);
