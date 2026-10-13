from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.device_binding_response import DeviceBindingResponse





T = TypeVar("T", bound="LineBindingResponse")



@_attrs_define
class LineBindingResponse:
    """ Line binding response.

        Attributes:
            id (str):
            phone_number_id (str):
            binding_type (str):
            devices (list[DeviceBindingResponse]):
            ring_strategy (str):
            tenant_id (str):
            shared (bool): A shared line and its appearances (`docs/ENTERPRISE_CALLING.md` §3.9).
            max_appearances (int):
            barge_in (bool): A phone on the shared line may barge into an active appearance
                (§3.11).
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            owner_user_id (None | str | Unset):
            outbound_caller_id (None | str | Unset):
            outbound_cnam (None | str | Unset):
            flow_id (None | str | Unset):
            schedule_id (None | str | Unset): The number's schedule, and where calls go when it is closed or on a
                holiday (`docs/ENTERPRISE_CALLING.md` §3.5).
            closed_target (None | str | Unset):
            holiday_target (None | str | Unset):
            hold_music (None | str | Unset): What a caller to this number hears on hold unless the holder chose
                their own: a prompt id, or `none` for silence; `null` for the
                tenant's hold music (`docs/ENTERPRISE_CALLING.md` §3.3).
     """

    id: str
    phone_number_id: str
    binding_type: str
    devices: list[DeviceBindingResponse]
    ring_strategy: str
    tenant_id: str
    shared: bool
    max_appearances: int
    barge_in: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    owner_user_id: None | str | Unset = UNSET
    outbound_caller_id: None | str | Unset = UNSET
    outbound_cnam: None | str | Unset = UNSET
    flow_id: None | str | Unset = UNSET
    schedule_id: None | str | Unset = UNSET
    closed_target: None | str | Unset = UNSET
    holiday_target: None | str | Unset = UNSET
    hold_music: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.device_binding_response import DeviceBindingResponse # noqa: PLC0415
        id = self.id

        phone_number_id = self.phone_number_id

        binding_type = self.binding_type

        devices = []
        for devices_item_data in self.devices:
            devices_item = devices_item_data.to_dict()
            devices.append(devices_item)



        ring_strategy = self.ring_strategy

        tenant_id = self.tenant_id

        shared = self.shared

        max_appearances = self.max_appearances

        barge_in = self.barge_in

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        owner_user_id: None | str | Unset
        if isinstance(self.owner_user_id, Unset):
            owner_user_id = UNSET
        else:
            owner_user_id = self.owner_user_id

        outbound_caller_id: None | str | Unset
        if isinstance(self.outbound_caller_id, Unset):
            outbound_caller_id = UNSET
        else:
            outbound_caller_id = self.outbound_caller_id

        outbound_cnam: None | str | Unset
        if isinstance(self.outbound_cnam, Unset):
            outbound_cnam = UNSET
        else:
            outbound_cnam = self.outbound_cnam

        flow_id: None | str | Unset
        if isinstance(self.flow_id, Unset):
            flow_id = UNSET
        else:
            flow_id = self.flow_id

        schedule_id: None | str | Unset
        if isinstance(self.schedule_id, Unset):
            schedule_id = UNSET
        else:
            schedule_id = self.schedule_id

        closed_target: None | str | Unset
        if isinstance(self.closed_target, Unset):
            closed_target = UNSET
        else:
            closed_target = self.closed_target

        holiday_target: None | str | Unset
        if isinstance(self.holiday_target, Unset):
            holiday_target = UNSET
        else:
            holiday_target = self.holiday_target

        hold_music: None | str | Unset
        if isinstance(self.hold_music, Unset):
            hold_music = UNSET
        else:
            hold_music = self.hold_music


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "phone_number_id": phone_number_id,
            "binding_type": binding_type,
            "devices": devices,
            "ring_strategy": ring_strategy,
            "tenant_id": tenant_id,
            "shared": shared,
            "max_appearances": max_appearances,
            "barge_in": barge_in,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if owner_user_id is not UNSET:
            field_dict["owner_user_id"] = owner_user_id
        if outbound_caller_id is not UNSET:
            field_dict["outbound_caller_id"] = outbound_caller_id
        if outbound_cnam is not UNSET:
            field_dict["outbound_cnam"] = outbound_cnam
        if flow_id is not UNSET:
            field_dict["flow_id"] = flow_id
        if schedule_id is not UNSET:
            field_dict["schedule_id"] = schedule_id
        if closed_target is not UNSET:
            field_dict["closed_target"] = closed_target
        if holiday_target is not UNSET:
            field_dict["holiday_target"] = holiday_target
        if hold_music is not UNSET:
            field_dict["hold_music"] = hold_music

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.device_binding_response import DeviceBindingResponse # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        phone_number_id = d.pop("phone_number_id")

        binding_type = d.pop("binding_type")

        devices = []
        _devices = d.pop("devices")
        for devices_item_data in (_devices):
            devices_item = DeviceBindingResponse.from_dict(devices_item_data)



            devices.append(devices_item)


        ring_strategy = d.pop("ring_strategy")

        tenant_id = d.pop("tenant_id")

        shared = d.pop("shared")

        max_appearances = d.pop("max_appearances")

        barge_in = d.pop("barge_in")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_owner_user_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_user_id = _parse_owner_user_id(d.pop("owner_user_id", UNSET))


        def _parse_outbound_caller_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        outbound_caller_id = _parse_outbound_caller_id(d.pop("outbound_caller_id", UNSET))


        def _parse_outbound_cnam(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        outbound_cnam = _parse_outbound_cnam(d.pop("outbound_cnam", UNSET))


        def _parse_flow_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        flow_id = _parse_flow_id(d.pop("flow_id", UNSET))


        def _parse_schedule_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        schedule_id = _parse_schedule_id(d.pop("schedule_id", UNSET))


        def _parse_closed_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        closed_target = _parse_closed_target(d.pop("closed_target", UNSET))


        def _parse_holiday_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        holiday_target = _parse_holiday_target(d.pop("holiday_target", UNSET))


        def _parse_hold_music(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        hold_music = _parse_hold_music(d.pop("hold_music", UNSET))


        line_binding_response = cls(
            id=id,
            phone_number_id=phone_number_id,
            binding_type=binding_type,
            devices=devices,
            ring_strategy=ring_strategy,
            tenant_id=tenant_id,
            shared=shared,
            max_appearances=max_appearances,
            barge_in=barge_in,
            created_at=created_at,
            updated_at=updated_at,
            owner_user_id=owner_user_id,
            outbound_caller_id=outbound_caller_id,
            outbound_cnam=outbound_cnam,
            flow_id=flow_id,
            schedule_id=schedule_id,
            closed_target=closed_target,
            holiday_target=holiday_target,
            hold_music=hold_music,
        )


        line_binding_response.additional_properties = d
        return line_binding_response

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
