import os
import socket

import pylspclient

from dataclasses import dataclass


@dataclass
class LSPConnectOptions:
    addr: str = '127.0.0.1'
    port: int = 8002
    root_uri = 'file:///workspace'


class LSPCall:
    def __init__(
            self,
            connect_options: LSPConnectOptions
    ):
        self._connect_options = connect_options

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.shutdown()

    def load_document(
            self,
            file_name: str,
            text: str,
            version: int = 1,
            language: str = 'python'
    ):
        if language == 'python':
            languageId = pylspclient.lsp_structs.LANGUAGE_IDENTIFIER.PYTHON  # noqa;
        else:
            raise NotImplemented(f"language {language} is not implemented for LSPCall.load_document")
        uri = os.path.join(self._connect_options.root_uri, file_name)
        self._lsp_client.didOpen(pylspclient.lsp_structs.TextDocumentItem(uri, languageId, version, text=text))

    def connect(self):
        self._connect2lsp(self._connect_options)

    def shutdown(self):
        print('LSPCall is shutting down...')
        try:
            self._lsp_client.shutdown()
            self._lsp_endpoint.join()
        except Exception:
            pass

    def _connect2lsp(self, connect_options: LSPConnectOptions):
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.connect(
            (connect_options.addr, connect_options.port)
        )
        pipe_in, pipe_out = s.makefile("wb", buffering=0), s.makefile("rb", buffering=0)
        json_rpc_endpoint = pylspclient.JsonRpcEndpoint(pipe_in, pipe_out)
        self._lsp_endpoint = pylspclient.LspEndpoint(json_rpc_endpoint)
        self._lsp_client = pylspclient.LspClient(self._lsp_endpoint)
        capabilities = {}
        workspace_folders = [{'name': 'workspace', 'uri': connect_options.root_uri}]
        self._lsp_client.initialize(
            1337,
            None,
            connect_options.root_uri,
            None,
            capabilities,
            "off",
            workspace_folders
        )
