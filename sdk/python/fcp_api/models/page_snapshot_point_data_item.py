from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageSnapshotPointDataItem")



@_attrs_define
class PageSnapshotPointDataItem:
    """ A run's snapshots, as one point a restore or a download takes.

        Attributes:
            run (str): The run that made it (`bkr-<uuid>`).
            time (str):
            host (str):
            volumes (list[str]): The volumes the files hold.
            release (None | str | Unset):
            kek_id (None | str | Unset):
            database (None | str | Unset): restic's ids of the database's snapshot and the files'.
            files (None | str | Unset):
     """

    run: str
    time: str
    host: str
    volumes: list[str]
    release: None | str | Unset = UNSET
    kek_id: None | str | Unset = UNSET
    database: None | str | Unset = UNSET
    files: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        run = self.run

        time = self.time

        host = self.host

        volumes = self.volumes



        release: None | str | Unset
        if isinstance(self.release, Unset):
            release = UNSET
        else:
            release = self.release

        kek_id: None | str | Unset
        if isinstance(self.kek_id, Unset):
            kek_id = UNSET
        else:
            kek_id = self.kek_id

        database: None | str | Unset
        if isinstance(self.database, Unset):
            database = UNSET
        else:
            database = self.database

        files: None | str | Unset
        if isinstance(self.files, Unset):
            files = UNSET
        else:
            files = self.files


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "run": run,
            "time": time,
            "host": host,
            "volumes": volumes,
        })
        if release is not UNSET:
            field_dict["release"] = release
        if kek_id is not UNSET:
            field_dict["kek_id"] = kek_id
        if database is not UNSET:
            field_dict["database"] = database
        if files is not UNSET:
            field_dict["files"] = files

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        run = d.pop("run")

        time = d.pop("time")

        host = d.pop("host")

        volumes = cast(list[str], d.pop("volumes"))


        def _parse_release(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        release = _parse_release(d.pop("release", UNSET))


        def _parse_kek_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kek_id = _parse_kek_id(d.pop("kek_id", UNSET))


        def _parse_database(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        database = _parse_database(d.pop("database", UNSET))


        def _parse_files(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        files = _parse_files(d.pop("files", UNSET))


        page_snapshot_point_data_item = cls(
            run=run,
            time=time,
            host=host,
            volumes=volumes,
            release=release,
            kek_id=kek_id,
            database=database,
            files=files,
        )


        page_snapshot_point_data_item.additional_properties = d
        return page_snapshot_point_data_item

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
