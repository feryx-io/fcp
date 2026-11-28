from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.backup_run_kind import BackupRunKind
from ..models.backup_run_state import BackupRunState
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RunView")



@_attrs_define
class RunView:
    """ A run of a destination.

        Attributes:
            id (str): `bkr-<uuid>`.
            destination_id (str):
            kind (BackupRunKind): What a run did.
            state (BackupRunState): Where a run is.
            trigger (str):
            requested_at (str):
            snapshots (list[str]):
            bytes_added (int):
            started_at (None | str | Unset):
            finished_at (None | str | Unset):
            message (None | str | Unset):
            node (None | str | Unset):
     """

    id: str
    destination_id: str
    kind: BackupRunKind
    state: BackupRunState
    trigger: str
    requested_at: str
    snapshots: list[str]
    bytes_added: int
    started_at: None | str | Unset = UNSET
    finished_at: None | str | Unset = UNSET
    message: None | str | Unset = UNSET
    node: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        destination_id = self.destination_id

        kind = self.kind.value

        state = self.state.value

        trigger = self.trigger

        requested_at = self.requested_at

        snapshots = self.snapshots



        bytes_added = self.bytes_added

        started_at: None | str | Unset
        if isinstance(self.started_at, Unset):
            started_at = UNSET
        else:
            started_at = self.started_at

        finished_at: None | str | Unset
        if isinstance(self.finished_at, Unset):
            finished_at = UNSET
        else:
            finished_at = self.finished_at

        message: None | str | Unset
        if isinstance(self.message, Unset):
            message = UNSET
        else:
            message = self.message

        node: None | str | Unset
        if isinstance(self.node, Unset):
            node = UNSET
        else:
            node = self.node


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "destination_id": destination_id,
            "kind": kind,
            "state": state,
            "trigger": trigger,
            "requested_at": requested_at,
            "snapshots": snapshots,
            "bytes_added": bytes_added,
        })
        if started_at is not UNSET:
            field_dict["started_at"] = started_at
        if finished_at is not UNSET:
            field_dict["finished_at"] = finished_at
        if message is not UNSET:
            field_dict["message"] = message
        if node is not UNSET:
            field_dict["node"] = node

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        destination_id = d.pop("destination_id")

        kind = BackupRunKind(d.pop("kind"))




        state = BackupRunState(d.pop("state"))




        trigger = d.pop("trigger")

        requested_at = d.pop("requested_at")

        snapshots = cast(list[str], d.pop("snapshots"))


        bytes_added = d.pop("bytes_added")

        def _parse_started_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        started_at = _parse_started_at(d.pop("started_at", UNSET))


        def _parse_finished_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        finished_at = _parse_finished_at(d.pop("finished_at", UNSET))


        def _parse_message(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        message = _parse_message(d.pop("message", UNSET))


        def _parse_node(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        node = _parse_node(d.pop("node", UNSET))


        run_view = cls(
            id=id,
            destination_id=destination_id,
            kind=kind,
            state=state,
            trigger=trigger,
            requested_at=requested_at,
            snapshots=snapshots,
            bytes_added=bytes_added,
            started_at=started_at,
            finished_at=finished_at,
            message=message,
            node=node,
        )


        run_view.additional_properties = d
        return run_view

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
