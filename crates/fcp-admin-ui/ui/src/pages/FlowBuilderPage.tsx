import { useState, useCallback, useEffect } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  ReactFlow,
  Controls,
  Background,
  addEdge,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
  BackgroundVariant,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  Save, ArrowLeft, Trash2, Play, Mic, Phone,
  Voicemail, GitBranch, PhoneOff, Globe, Users, KeyRound,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { flows } from '@/api/endpoints'
import type { FlowNode } from '@/types'

const NODE_TYPES_PALETTE = [
  { type: 'answer', label: 'Answer', icon: Phone, color: '#22c55e' },
  { type: 'say', label: 'Say', icon: Mic, color: '#3b82f6' },
  { type: 'play', label: 'Play', icon: Play, color: '#8b5cf6' },
  { type: 'gather', label: 'Gather', icon: Phone, color: '#10b981' },
  { type: 'dial', label: 'Dial', icon: Phone, color: '#f59e0b' },
  { type: 'queue', label: 'Queue', icon: Users, color: '#6366f1' },
  { type: 'voicemail', label: 'Voicemail', icon: Voicemail, color: '#ec4899' },
  { type: 'voicemail_access', label: 'VM Access', icon: KeyRound, color: '#d946ef' },
  { type: 'branch', label: 'Branch', icon: GitBranch, color: '#14b8a6' },
  { type: 'http', label: 'HTTP', icon: Globe, color: '#f97316' },
  { type: 'hangup', label: 'Hangup', icon: PhoneOff, color: '#ef4444' },
]

// Fields that should not appear as editable properties
const HIDDEN_FIELDS = new Set(['type', 'label', 'id', 'name'])

// Fields that reference other node IDs (rendered as node selectors)
const NODE_REF_FIELDS = new Set([
  'next', 'on_error', 'on_busy', 'on_no_answer', 'on_timeout',
  'default', 'if_true', 'if_false', 'next_on_success', 'next_on_error',
  'on_auth_failure',
])

// Boolean fields
const BOOL_FIELDS = new Set(['is_busy', 'play_beep'])

// Numeric fields
const NUMERIC_FIELDS = new Set([
  'loop_count', 'num_digits', 'timeout', 'max_duration',
  'max_pin_attempts', 'duration',
])

// Select field options
const SELECT_OPTIONS: Record<string, { label: string; value: string }[]> = {
  method: [
    { label: 'GET', value: 'GET' },
    { label: 'POST', value: 'POST' },
    { label: 'PUT', value: 'PUT' },
    { label: 'DELETE', value: 'DELETE' },
    { label: 'PATCH', value: 'PATCH' },
  ],
  operator: [
    { label: 'Equals', value: 'equals' },
    { label: 'Not Equals', value: 'not_equals' },
    { label: 'Contains', value: 'contains' },
    { label: 'Starts With', value: 'starts_with' },
    { label: 'Ends With', value: 'ends_with' },
    { label: 'Greater Than', value: 'greater_than' },
    { label: 'Less Than', value: 'less_than' },
    { label: 'Exists', value: 'exists' },
    { label: 'Not Exists', value: 'not_exists' },
  ],
  strategy: [
    { label: 'First', value: 'first' },
    { label: 'All', value: 'all' },
  ],
  format: [
    { label: 'WAV', value: 'wav' },
    { label: 'MP3', value: 'mp3' },
  ],
  level: [
    { label: 'Debug', value: 'debug' },
    { label: 'Info', value: 'info' },
    { label: 'Warn', value: 'warn' },
    { label: 'Error', value: 'error' },
  ],
}

// Human-readable labels for fields
const FIELD_LABELS: Record<string, string> = {
  next: 'Next Node',
  on_error: 'On Error',
  on_busy: 'On Busy',
  on_no_answer: 'On No Answer',
  on_timeout: 'On Timeout',
  on_auth_failure: 'On Auth Failure',
  default: 'Default',
  if_true: 'If True',
  if_false: 'If False',
  next_on_success: 'On Success',
  next_on_error: 'On Error',
  text: 'Text',
  url: 'URL',
  voice: 'Voice',
  language: 'Language',
  destination: 'Destination',
  caller_id: 'Caller ID',
  timeout: 'Timeout (sec)',
  max_duration: 'Max Duration (sec)',
  loop_count: 'Loop Count',
  num_digits: 'Num Digits',
  finish_on_key: 'Finish On Key',
  variable: 'Variable',
  value: 'Value',
  operator: 'Operator',
  method: 'Method',
  body: 'Body',
  response_variable: 'Response Variable',
  mailbox: 'Mailbox',
  is_busy: 'Busy Greeting',
  play_beep: 'Play Beep',
  max_pin_attempts: 'Max PIN Attempts',
  message: 'Message',
  level: 'Log Level',
  strategy: 'Strategy',
  format: 'Format',
  reason: 'Reason',
  duration: 'Duration (sec)',
  transitions: 'DTMF Transitions',
  branches: 'Branches',
  headers: 'Headers',
  destinations: 'Destinations',
  prompt: 'Prompt',
}

function flowNodesToReactFlow(
  flowNodes: Record<string, FlowNode>,
  initialNode: string,
): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = []
  const edges: Edge[] = []
  const entries = Object.entries(flowNodes)

  entries.forEach(([id, node], index) => {
    const palette = NODE_TYPES_PALETTE.find((p) => p.type === node.type)
    nodes.push({
      id,
      type: 'default',
      position: { x: 250, y: index * 120 },
      data: {
        label: (
          <div className="text-xs">
            <div className="font-bold" style={{ color: palette?.color }}>
              {node.label || id}
            </div>
            <div className="text-muted-foreground">{node.type}</div>
          </div>
        ),
      },
      style: {
        border: `2px solid ${palette?.color ?? '#888'}`,
        borderRadius: 8,
        padding: 8,
        background: 'var(--card)',
      },
    })

    // Create edges from "next" field
    const next = (node as Record<string, unknown>).next as string | null
    if (next && flowNodes[next]) {
      edges.push({
        id: `${id}->${next}`,
        source: id,
        target: next,
        markerEnd: { type: MarkerType.ArrowClosed },
        animated: id === initialNode,
      })
    }

    // Branch edges from transitions (gather node)
    const transitions = (node as Record<string, unknown>).transitions as
      | Record<string, string>
      | undefined
    if (transitions) {
      Object.entries(transitions).forEach(([key, target]) => {
        if (flowNodes[target]) {
          edges.push({
            id: `${id}->${target}[${key}]`,
            source: id,
            target,
            label: key,
            markerEnd: { type: MarkerType.ArrowClosed },
          })
        }
      })
    }

    // Branch edges from legacy "branches" field
    const branches = (node as Record<string, unknown>).branches as
      | Record<string, string>
      | undefined
    if (branches) {
      Object.entries(branches).forEach(([key, target]) => {
        if (flowNodes[target]) {
          edges.push({
            id: `${id}->${target}[${key}]`,
            source: id,
            target,
            label: key,
            markerEnd: { type: MarkerType.ArrowClosed },
          })
        }
      })
    }
  })

  return { nodes, edges }
}

function reactFlowToFlowNodes(
  nodes: Node[],
  edges: Edge[],
  existingFlowNodes: Record<string, FlowNode>,
): Record<string, FlowNode> {
  const result: Record<string, FlowNode> = {}

  nodes.forEach((node) => {
    const existing = existingFlowNodes[node.id] || { type: 'say' }
    const outgoing = edges.filter((e) => e.source === node.id)
    const mainEdge = outgoing.find((e) => !e.label)

    result[node.id] = {
      ...existing,
      next: mainEdge?.target ?? null,
    } as FlowNode & { next: string | null }
  })

  return result
}

export function FlowBuilderPage() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const { data: flow, isLoading } = useQuery({
    queryKey: ['flow', id],
    queryFn: () => flows.get(id!),
    enabled: !!id,
  })

  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([])
  const [selectedNode, setSelectedNode] = useState<string | null>(null)
  const [flowName, setFlowName] = useState('')
  const [flowDescription, setFlowDescription] = useState('')
  const [nodeCounter, setNodeCounter] = useState(0)
  // Local mutable copy of flow nodes (source of truth for property editing)
  const [flowNodes, setFlowNodes] = useState<Record<string, FlowNode>>({})
  const [dirty, setDirty] = useState(false)

  // Initialize from flow data
  useEffect(() => {
    if (flow) {
      const nodesData = flow.nodes || {}
      const { nodes: rfNodes, edges: rfEdges } = flowNodesToReactFlow(
        nodesData,
        flow.initial_node,
      )
      setNodes(rfNodes)
      setEdges(rfEdges)
      setFlowName(flow.name)
      setFlowDescription(flow.description || '')
      setNodeCounter(Object.keys(nodesData).length)
      setFlowNodes(structuredClone(nodesData))
      setDirty(false)
    }
  }, [flow, setNodes, setEdges])

  const onConnect = useCallback(
    (connection: Connection) => {
      setEdges((eds) =>
        addEdge(
          {
            ...connection,
            markerEnd: { type: MarkerType.ArrowClosed },
          },
          eds,
        ),
      )
      setDirty(true)
    },
    [setEdges],
  )

  // Update a property on a flow node
  const updateNodeProperty = useCallback(
    (nodeId: string, key: string, value: unknown) => {
      setFlowNodes((prev) => ({
        ...prev,
        [nodeId]: {
          ...prev[nodeId],
          [key]: value,
        },
      }))
      setDirty(true)
    },
    [],
  )

  // Update a key in a transitions/branches map
  const updateMapEntry = useCallback(
    (nodeId: string, mapKey: string, dtmfKey: string, target: string) => {
      setFlowNodes((prev) => {
        const node = prev[nodeId]
        const map = ((node as Record<string, unknown>)[mapKey] as Record<string, string>) || {}
        return {
          ...prev,
          [nodeId]: {
            ...node,
            [mapKey]: { ...map, [dtmfKey]: target },
          },
        }
      })
      setDirty(true)
    },
    [],
  )

  const deleteMapEntry = useCallback(
    (nodeId: string, mapKey: string, dtmfKey: string) => {
      setFlowNodes((prev) => {
        const node = prev[nodeId]
        const map = { ...((node as Record<string, unknown>)[mapKey] as Record<string, string> || {}) }
        delete map[dtmfKey]
        return {
          ...prev,
          [nodeId]: {
            ...node,
            [mapKey]: map,
          },
        }
      })
      setDirty(true)
    },
    [],
  )

  const addMapEntry = useCallback(
    (nodeId: string, mapKey: string) => {
      setFlowNodes((prev) => {
        const node = prev[nodeId]
        const map = ((node as Record<string, unknown>)[mapKey] as Record<string, string>) || {}
        // Find next available key
        const usedKeys = Object.keys(map)
        const allKeys = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '*', '#']
        const nextKey = allKeys.find((k) => !usedKeys.includes(k)) || `key_${usedKeys.length}`
        return {
          ...prev,
          [nodeId]: {
            ...node,
            [mapKey]: { ...map, [nextKey]: '' },
          },
        }
      })
      setDirty(true)
    },
    [],
  )

  const saveMutation = useMutation({
    mutationFn: () => {
      // Merge local flowNodes with edge-derived connections
      const mergedNodes = reactFlowToFlowNodes(nodes, edges, flowNodes)
      return flows.update(id!, {
        name: flowName,
        description: flowDescription || undefined,
        nodes: mergedNodes,
      })
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['flow', id] })
      queryClient.invalidateQueries({ queryKey: ['flows'] })
      setDirty(false)
    },
  })

  const addNode = (type: string) => {
    const palette = NODE_TYPES_PALETTE.find((p) => p.type === type)
    const newId = `${type}_${nodeCounter}`
    setNodeCounter((c) => c + 1)

    // Create default flow node data based on type
    const defaults: Record<string, Record<string, unknown>> = {
      answer: { next: '' },
      say: { text: '', language: 'en-US', next: '' },
      play: { url: '', loop_count: 0, next: '' },
      gather: { num_digits: 1, timeout: 5, variable: 'digits', transitions: {}, next: '' },
      dial: { destination: '', timeout: 30, max_duration: 0, next: '' },
      hangup: { reason: '' },
      http: { method: 'GET', url: '', timeout: 10, response_variable: 'http_response', next: '' },
      branch: { variable: '', operator: 'equals', value: '', if_true: '', if_false: '' },
      voicemail: { mailbox: '', is_busy: false, max_duration: 300, play_beep: true, next_on_success: '' },
      voicemail_access: { mailbox: '', max_pin_attempts: 3, next: '' },
      queue: { next: '' },
    }

    const newFlowNode: FlowNode = {
      type,
      label: newId,
      ...(defaults[type] || { next: '' }),
    } as FlowNode

    setFlowNodes((prev) => ({ ...prev, [newId]: newFlowNode }))

    const newNode: Node = {
      id: newId,
      type: 'default',
      position: { x: 300 + Math.random() * 100, y: 100 + nodes.length * 80 },
      data: {
        label: (
          <div className="text-xs">
            <div className="font-bold" style={{ color: palette?.color }}>
              {newId}
            </div>
            <div className="text-muted-foreground">{type}</div>
          </div>
        ),
      },
      style: {
        border: `2px solid ${palette?.color ?? '#888'}`,
        borderRadius: 8,
        padding: 8,
        background: 'var(--card)',
      },
    }
    setNodes((nds) => [...nds, newNode])
    setDirty(true)
  }

  const deleteSelectedNode = () => {
    if (!selectedNode) return
    setNodes((nds) => nds.filter((n) => n.id !== selectedNode))
    setEdges((eds) =>
      eds.filter((e) => e.source !== selectedNode && e.target !== selectedNode),
    )
    setFlowNodes((prev) => {
      const copy = { ...prev }
      delete copy[selectedNode]
      return copy
    })
    setSelectedNode(null)
    setDirty(true)
  }

  // Get all node IDs for the node reference selectors
  const allNodeIds = Object.keys(flowNodes)

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        Loading flow...
      </div>
    )
  }

  const selectedFlowNode = selectedNode ? flowNodes[selectedNode] : null

  // Render the editable property field for a given key/value
  const renderPropertyField = (nodeId: string, key: string, value: unknown) => {
    const label = FIELD_LABELS[key] || key

    // Map fields (transitions, branches, headers) - render as key/value editor
    if (key === 'transitions' || key === 'branches') {
      const map = (value as Record<string, string>) || {}
      return (
        <div key={key}>
          <Label className="text-xs font-medium">{label}</Label>
          <div className="mt-1 space-y-1">
            {Object.entries(map).map(([dtmfKey, target]) => (
              <div key={dtmfKey} className="flex items-center gap-1">
                <Input
                  value={dtmfKey}
                  className="h-7 w-12 text-xs font-mono text-center"
                  readOnly
                />
                <Select
                  value={target}
                  onValueChange={(v) => updateMapEntry(nodeId, key, dtmfKey, v)}
                >
                  <SelectTrigger className="h-7 text-xs flex-1">
                    <SelectValue placeholder="Select node..." />
                  </SelectTrigger>
                  <SelectContent>
                    {allNodeIds
                      .filter((nid) => nid !== nodeId)
                      .map((nid) => (
                        <SelectItem key={nid} value={nid}>
                          {nid}
                        </SelectItem>
                      ))}
                  </SelectContent>
                </Select>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7 shrink-0"
                  onClick={() => deleteMapEntry(nodeId, key, dtmfKey)}
                >
                  <Trash2 className="h-3 w-3" />
                </Button>
              </div>
            ))}
            <Button
              variant="outline"
              size="sm"
              className="w-full h-7 text-xs"
              onClick={() => addMapEntry(nodeId, key)}
            >
              + Add
            </Button>
          </div>
        </div>
      )
    }

    // Nested objects (prompt, headers, destinations) - render as JSON for now
    if (key === 'headers' || key === 'destinations' || key === 'prompt') {
      const jsonStr = typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value ?? '')
      return (
        <div key={key}>
          <Label className="text-xs font-medium">{label}</Label>
          <textarea
            value={jsonStr}
            onChange={(e) => {
              try {
                const parsed = JSON.parse(e.target.value)
                updateNodeProperty(nodeId, key, parsed)
              } catch {
                // Allow invalid JSON while typing - store raw string
              }
            }}
            className="mt-1 w-full h-20 rounded-md border border-input bg-background px-2 py-1 text-xs font-mono resize-y"
          />
        </div>
      )
    }

    // Node reference fields - render as select dropdown
    if (NODE_REF_FIELDS.has(key)) {
      return (
        <div key={key}>
          <Label className="text-xs font-medium">{label}</Label>
          <Select
            value={String(value ?? '')}
            onValueChange={(v) => updateNodeProperty(nodeId, key, v)}
          >
            <SelectTrigger className="mt-1 h-8 text-xs">
              <SelectValue placeholder="Select node..." />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="">
                <span className="text-muted-foreground">None</span>
              </SelectItem>
              {allNodeIds
                .filter((nid) => nid !== nodeId)
                .map((nid) => (
                  <SelectItem key={nid} value={nid}>
                    {nid}
                  </SelectItem>
                ))}
            </SelectContent>
          </Select>
        </div>
      )
    }

    // Select fields (method, operator, etc.)
    if (SELECT_OPTIONS[key]) {
      return (
        <div key={key}>
          <Label className="text-xs font-medium">{label}</Label>
          <Select
            value={String(value ?? '')}
            onValueChange={(v) => updateNodeProperty(nodeId, key, v)}
          >
            <SelectTrigger className="mt-1 h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {SELECT_OPTIONS[key].map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )
    }

    // Boolean fields
    if (BOOL_FIELDS.has(key)) {
      return (
        <div key={key} className="flex items-center justify-between">
          <Label className="text-xs font-medium">{label}</Label>
          <Switch
            checked={!!value}
            onCheckedChange={(v) => updateNodeProperty(nodeId, key, v)}
          />
        </div>
      )
    }

    // Numeric fields
    if (NUMERIC_FIELDS.has(key)) {
      return (
        <div key={key}>
          <Label className="text-xs font-medium">{label}</Label>
          <Input
            type="number"
            value={String(value ?? 0)}
            onChange={(e) =>
              updateNodeProperty(nodeId, key, parseInt(e.target.value) || 0)
            }
            className="mt-1 h-8 text-xs"
          />
        </div>
      )
    }

    // Text fields (default)
    // Use textarea for long text fields
    if (key === 'text' || key === 'body' || key === 'message') {
      return (
        <div key={key}>
          <Label className="text-xs font-medium">{label}</Label>
          <textarea
            value={String(value ?? '')}
            onChange={(e) => updateNodeProperty(nodeId, key, e.target.value)}
            className="mt-1 w-full h-16 rounded-md border border-input bg-background px-2 py-1 text-xs resize-y"
          />
        </div>
      )
    }

    return (
      <div key={key}>
        <Label className="text-xs font-medium">{label}</Label>
        <Input
          value={String(value ?? '')}
          onChange={(e) => updateNodeProperty(nodeId, key, e.target.value)}
          className="mt-1 h-8 text-xs"
        />
      </div>
    )
  }

  return (
    <div className="flex flex-col h-[calc(100vh-7rem)]">
      {/* Toolbar */}
      <div className="flex items-center justify-between border-b bg-card px-4 py-2">
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => navigate('/admin/flows')}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <Input
            value={flowName}
            onChange={(e) => {
              setFlowName(e.target.value)
              setDirty(true)
            }}
            className="w-48 h-8 text-sm font-medium"
          />
          <Badge variant="outline">v{flow?.version ?? 1}</Badge>
          <Badge variant={flow?.enabled ? 'success' : 'secondary'}>
            {flow?.enabled ? 'Active' : 'Disabled'}
          </Badge>
          {dirty && (
            <Badge variant="outline" className="text-yellow-500 border-yellow-500">
              Unsaved
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-2">
          {selectedNode && (
            <Button variant="outline" size="sm" onClick={deleteSelectedNode}>
              <Trash2 className="mr-1 h-3 w-3" />
              Delete Node
            </Button>
          )}
          <Button
            size="sm"
            onClick={() => saveMutation.mutate()}
            disabled={saveMutation.isPending}
          >
            <Save className="mr-1 h-3 w-3" />
            {saveMutation.isPending ? 'Saving...' : 'Save'}
          </Button>
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">
        {/* Node Palette */}
        <div className="w-48 border-r bg-card p-3 overflow-y-auto">
          <p className="text-xs font-semibold text-muted-foreground mb-2 uppercase">
            Add Node
          </p>
          <div className="space-y-1">
            {NODE_TYPES_PALETTE.map((item) => {
              const Icon = item.icon
              return (
                <Button
                  key={item.type}
                  variant="ghost"
                  size="sm"
                  className="w-full justify-start gap-2 text-xs"
                  onClick={() => addNode(item.type)}
                >
                  <Icon className="h-3 w-3" style={{ color: item.color }} />
                  {item.label}
                </Button>
              )
            })}
          </div>
        </div>

        {/* React Flow Canvas */}
        <div className="flex-1">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={(_, node) => setSelectedNode(node.id)}
            onPaneClick={() => setSelectedNode(null)}
            fitView
          >
            <Controls />
            <Background variant={BackgroundVariant.Dots} gap={16} />
          </ReactFlow>
        </div>

        {/* Properties Panel */}
        {selectedNode && selectedFlowNode && (
          <div className="w-72 border-l bg-card p-4 overflow-y-auto">
            <h3 className="font-semibold text-sm mb-3">Node Properties</h3>
            <div className="space-y-3 text-sm">
              {/* Node ID (read-only) */}
              <div>
                <Label className="text-xs font-medium">ID</Label>
                <p className="font-mono text-xs bg-muted rounded px-2 py-1 mt-1">
                  {selectedNode}
                </p>
              </div>

              {/* Node Type (read-only) */}
              <div>
                <Label className="text-xs font-medium">Type</Label>
                <p className="capitalize text-xs bg-muted rounded px-2 py-1 mt-1">
                  {selectedFlowNode.type}
                </p>
              </div>

              {/* Label (editable) */}
              <div>
                <Label className="text-xs font-medium">Label</Label>
                <Input
                  value={String(selectedFlowNode.label ?? '')}
                  onChange={(e) =>
                    updateNodeProperty(selectedNode, 'label', e.target.value)
                  }
                  className="mt-1 h-8 text-xs"
                  placeholder="Node label"
                />
              </div>

              {/* Separator */}
              <div className="border-t pt-2" />

              {/* All other properties */}
              {Object.entries(selectedFlowNode as Record<string, unknown>)
                .filter(([k]) => !HIDDEN_FIELDS.has(k))
                .map(([key, value]) => renderPropertyField(selectedNode, key, value))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
