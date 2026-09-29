from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.video_engines import VideoEngines





T = TypeVar("T", bound="VideoDeviceBudget")



@_attrs_define
class VideoDeviceBudget:
    """ One device's video budget on a node, in work units: one unit is a
    thousandth of one execution unit's time per second (on the host, a
    millicore of one codec-pool thread). `left` is what a new room or
    participant may still cost here.

        Attributes:
            device (str): `host`, or a GPU as `backend:address`.
            codecs (list[str]): SDP names of the codecs the device *encodes* (`H264`, `VP8`, …).
            left (int): What a new room or participant may still cost here: on a device
                with engines, the tightest engine's.
            total (int):
            engines (None | Unset | VideoEngines):
     """

    device: str
    codecs: list[str]
    left: int
    total: int
    engines: None | Unset | VideoEngines = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.video_engines import VideoEngines # noqa: PLC0415
        device = self.device

        codecs = self.codecs



        left = self.left

        total = self.total

        engines: dict[str, Any] | None | Unset
        if isinstance(self.engines, Unset):
            engines = UNSET
        elif isinstance(self.engines, VideoEngines):
            engines = self.engines.to_dict()
        else:
            engines = self.engines


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "device": device,
            "codecs": codecs,
            "left": left,
            "total": total,
        })
        if engines is not UNSET:
            field_dict["engines"] = engines

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.video_engines import VideoEngines # noqa: PLC0415
        d = dict(src_dict)
        device = d.pop("device")

        codecs = cast(list[str], d.pop("codecs"))


        left = d.pop("left")

        total = d.pop("total")

        def _parse_engines(data: object) -> None | Unset | VideoEngines:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                engines_type_0 = VideoEngines.from_dict(data)



                return engines_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | Unset | VideoEngines, data)

        engines = _parse_engines(d.pop("engines", UNSET))


        video_device_budget = cls(
            device=device,
            codecs=codecs,
            left=left,
            total=total,
            engines=engines,
        )


        video_device_budget.additional_properties = d
        return video_device_budget

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
