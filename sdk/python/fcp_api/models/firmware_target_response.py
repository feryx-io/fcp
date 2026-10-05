from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.rollout_stage import RolloutStage
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="FirmwareTargetResponse")



@_attrs_define
class FirmwareTargetResponse:
    """ A tenant's target.

        Attributes:
            id (str):
            vendor (str):
            image_id (str):
            stage (RolloutStage): How far a target has rolled out.
            pilot_devices (list[str]):
            devices (int): The tenant's phones it is for (stage aside), and where they stand.
            current (int):
            behind (int):
            unknown (int):
            updated_at (datetime.datetime):
            model_pattern (None | str | Unset):
            version (None | str | Unset):
     """

    id: str
    vendor: str
    image_id: str
    stage: RolloutStage
    pilot_devices: list[str]
    devices: int
    current: int
    behind: int
    unknown: int
    updated_at: datetime.datetime
    model_pattern: None | str | Unset = UNSET
    version: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        vendor = self.vendor

        image_id = self.image_id

        stage = self.stage.value

        pilot_devices = self.pilot_devices



        devices = self.devices

        current = self.current

        behind = self.behind

        unknown = self.unknown

        updated_at = self.updated_at.isoformat()

        model_pattern: None | str | Unset
        if isinstance(self.model_pattern, Unset):
            model_pattern = UNSET
        else:
            model_pattern = self.model_pattern

        version: None | str | Unset
        if isinstance(self.version, Unset):
            version = UNSET
        else:
            version = self.version


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "vendor": vendor,
            "image_id": image_id,
            "stage": stage,
            "pilot_devices": pilot_devices,
            "devices": devices,
            "current": current,
            "behind": behind,
            "unknown": unknown,
            "updated_at": updated_at,
        })
        if model_pattern is not UNSET:
            field_dict["model_pattern"] = model_pattern
        if version is not UNSET:
            field_dict["version"] = version

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        vendor = d.pop("vendor")

        image_id = d.pop("image_id")

        stage = RolloutStage(d.pop("stage"))




        pilot_devices = cast(list[str], d.pop("pilot_devices"))


        devices = d.pop("devices")

        current = d.pop("current")

        behind = d.pop("behind")

        unknown = d.pop("unknown")

        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_model_pattern(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model_pattern = _parse_model_pattern(d.pop("model_pattern", UNSET))


        def _parse_version(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        version = _parse_version(d.pop("version", UNSET))


        firmware_target_response = cls(
            id=id,
            vendor=vendor,
            image_id=image_id,
            stage=stage,
            pilot_devices=pilot_devices,
            devices=devices,
            current=current,
            behind=behind,
            unknown=unknown,
            updated_at=updated_at,
            model_pattern=model_pattern,
            version=version,
        )


        firmware_target_response.additional_properties = d
        return firmware_target_response

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
