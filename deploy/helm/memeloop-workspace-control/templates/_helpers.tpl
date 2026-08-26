{{- define "mwc.name" -}}
{{- printf "mwc-%s" (required "installationId is required" .Values.installationId) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "mwc.labels" -}}
app.kubernetes.io/name: memeloop-workspace-control
app.kubernetes.io/instance: {{ include "mwc.name" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
workspace.memeloop.dev/owner-installation: {{ required "installationId is required" .Values.installationId | quote }}
{{- end -}}

{{- define "mwc.selectorLabels" -}}
app.kubernetes.io/name: memeloop-workspace-control
app.kubernetes.io/instance: {{ include "mwc.name" . }}
{{- end -}}

{{- define "mwc.validate" -}}
{{- if not (regexMatch "^[a-z0-9]([-a-z0-9]{0,18}[a-z0-9])?$" .Values.installationId) -}}
{{- fail "installationId must be a lower-case DNS label of at most 20 characters" -}}
{{- end -}}
{{- if not (has .Values.mode (list "sqlite" "postgresql")) -}}
{{- fail "mode must be sqlite or postgresql" -}}
{{- end -}}
{{- if and (eq .Values.mode "postgresql") (not .Values.database.postgresSecretName) -}}
{{- fail "database.postgresSecretName is required in postgresql mode" -}}
{{- end -}}
{{- if not .Values.secrets.encryptionSecretName -}}
{{- fail "secrets.encryptionSecretName is required" -}}
{{- end -}}
{{- if not .Values.secrets.internalAuthSecretName -}}
{{- fail "secrets.internalAuthSecretName is required" -}}
{{- end -}}
{{- if not .Values.workspace.ttydImage -}}
{{- fail "workspace.ttydImage must be an explicitly pinned image" -}}
{{- end -}}
{{- if and .Values.public.webShellDomain (not .Values.higress.extAuthPluginUrl) -}}
{{- fail "higress.extAuthPluginUrl is required when public.webShellDomain is configured" -}}
{{- end -}}
{{- if and .Values.public.webShellDomain (ne .Values.public.webShellOrigin (printf "https://%s" .Values.public.webShellDomain)) -}}
{{- fail "public.webShellOrigin must equal https://<public.webShellDomain>" -}}
{{- end -}}
{{- if and .Values.public.webShellOrigin (not .Values.public.webShellDomain) -}}
{{- fail "public.webShellDomain is required when public.webShellOrigin is configured" -}}
{{- end -}}
{{- if and .Values.jumpHost.enabled (not .Values.jumpHost.hostKeySecretName) -}}
{{- fail "jumpHost.hostKeySecretName is required when jumpHost is enabled" -}}
{{- end -}}
{{- end -}}
